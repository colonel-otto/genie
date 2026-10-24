//! Garrisoning: `UNIT_ORDER`'s garrison (action 5 on a building, `FUN_14161e370` ->
//! `FUN_141621220`), which sends each unit to a building that takes it in, nearest first, each
//! building up to its capacity, and gives each building's units, through the order splitter,
//! the garrison work on it (slot 0x7e8 with task filter 3); and a building's test whether it takes
//! a unit in (`FUN_14120eb10`).

use crate::GenieError;
use crate::checkpoint::cvttss2si;
use crate::fmath::{at_most_or_unordered, maxss};
use crate::masters::{Master, Masters, Task};
use crate::move_to::{Goal, MoveTo, Walk, start_walk_or_give_up, walk_or_give_up};
use crate::movement::footprint_gap_squared;
use crate::order::{BUILDING, GARRISON, Work};
use crate::tasks::Diplomacy;
use crate::turn::{Turns, World};

/// The squared footprint gap past which a garrison order sends no unit (`FUN_141621220` with 0:
/// no limit).
const FAR: f32 = crate::image::garrison::FAR;
/// The squared footprint gap within which a garrison order's target's neighbours shelter units
/// too (`FUN_14161e370` with `+0x1e`, `DAT_14330ebcc`, 100.0: ten tiles).
pub(crate) const NEARBY: f32 = crate::image::garrison::NEARBY;
/// The squared footprint gap within which the town bell reaches (its ring `FUN_14163a650`, its
/// end `FUN_14163ab80`, and `FUN_141621220` with `param_6` 1: `DAT_142ef3e70`, 625.0, 25 tiles).
/// An enter into an object the world holds asleep.
const ENTER_SLEEPING: GenieError = GenieError::unsupported("an enter into a sleeping object");
/// An enter command under an Artifact on top (`FUN_1410847e0`: the Enter becomes the
/// Artifact's sub-action), which is not modelled.
const ENTER_UNDER_ARTIFACT: GenieError =
    GenieError::unsupported("an enter command under an Artifact (its sub-action)");
/// An enter without its owner's masters.
pub(crate) const NO_MASTERS: GenieError = GenieError::unsupported("an enter without the masters");
pub(crate) const TOWN_BELL_REACH: f32 = crate::image::garrison::TOWN_BELL_REACH;

/// The shelters `FUN_141621220` sends units to: the buildings (its `param_3`), the transports
/// (`param_4`), and the count list (`param_5`) its pairs index by their shelter's index, a
/// transport's after the buildings. Its callers push the counts in their own order (the
/// buildings' scan near a target pushes one for each object in reach, gates and annexes too),
/// so a pair's count need not be its shelter's.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Shelters {
    /// The buildings.
    pub buildings: Vec<i32>,
    /// The transports.
    pub transports: Vec<i32>,
    /// The counts ([`Turns::garrisoned`], slot 0x498).
    pub counts: Vec<i32>,
}
/// A building takes units in only above this share of its master's hit points (`FUN_14120eb10`,
/// in doubles).
const LEAST_HP_SHARE: f64 = crate::image::garrison::LEAST_HP_SHARE;
/// The class of buildings, which a unit's garrison task names (`FUN_1415c9240` with 3, 3).
pub(crate) const BUILDING_CLASS: i16 = 3;
/// A gate's class: its annex is not its main building for a garrison order.
const GATE_CLASS: i16 = 39;
/// The ids a command's check reads (`FUN_140e558b0`): a human player's first 60, a computer
/// player's first 500.
const HUMAN_IDS: usize = 60;
const COMPUTER_IDS: usize = 500;
/// `SEEK_SHELTER`'s villagers: class 4 of type 70.
const VILLAGER_CLASS: i16 = 4;
const VILLAGER_KIND: u8 = 70;

/// The class-flags table `FUN_1411205e0` reads (`DAT_1439e37b0`, a u32 per class; its count,
/// `DAT_1439e2cf8`, is 64), as the image holds it (nothing writes it).
const CLASS_FLAGS: [u32; 64] = crate::image::garrison::CLASS_FLAGS;
/// The class flags a unit garrisons a transport with (`FUN_14120eb10`, `FUN_141621220`).
const TRANSPORTED: u32 = 0x10002;

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
pub(crate) const COMBAT_KIND: u8 = 70;

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
) -> Result<f32, GenieError> {
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
    ) -> Result<bool, GenieError> {
        let capacity = i32::from(self.master_of(bp, b.life.master_id)?.garrison);
        let with = match unit {
            Some(u) => self.garrisoned(u)? + 1,
            None => 1,
        };
        Ok(self.garrisoned(b.id)? + with <= capacity)
    }

    /// The work command's Enter (its action type 3, [`Turns::new_enter`]), the unit AI hearing of
    /// it (slot 0x828: the enter order and action, the target with its class and the work's
    /// point), then its install ([`Turns::install_enter`]).
    pub(crate) fn start_enter(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
        task: &Task,
        point: [f32; 3],
        clears: bool,
    ) -> Result<(), GenieError> {
        let (action, target, class) = self.new_enter(player, idx, target, task)?;
        if let Some(ai) = self.players[player].objects[idx].ai.as_mut() {
            ai.notify_work(
                ENTER_ORDER,
                ENTER_ACTION,
                Some((target, class)),
                point,
                clears,
                self.streams.get(crate::rng::Stream::Ai),
            )?;
            self.remember_order(player, idx, (ENTER_ORDER, ENTER_ACTION))?;
        }
        self.install_enter(player, idx, target, action, task)
    }

    /// Slot 0x3a0 (`FUN_1410847e0`, the enter command) of the unit at `idx` of `player` on
    /// `target`, `forced` its third argument: a target the world does not hold is false;
    /// unforced, a unit entering the target already (its top action an Enter on it) is true.
    /// Slot 0x820 ([`Turns::find_work_task`]) looks for a task on the target with the garrison
    /// filter (3) and the switch (the unit takes the master that gave it, slot 0x90): none is
    /// false; else its Enter ([`Turns::enter_with`]): true.
    ///
    /// # Errors
    ///
    /// What the Enter does not model.
    pub(crate) fn enter_command(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
        forced: bool,
    ) -> Result<bool, GenieError> {
        if self.object(target).is_none() {
            return Ok(false);
        }
        let unit = &self.players[player].objects[idx];
        let top = unit.top_action_kind();
        if !forced && top == Some(ENTER) && unit.top_target() == Some(target) {
            return Ok(true);
        }
        let Some(task) = self.find_work_task(player, idx, target, GARRISON, true)? else {
            return Ok(false);
        };
        let under_artifact = top == Some(crate::artifact::ARTIFACT_TYPE);
        self.enter_with(player, idx, target, &task, under_artifact)?;
        Ok(true)
    }

    /// The unit's enter command its unit AI gave ([`crate::turn::EnterOrder`]), after the
    /// update: the master switch (slot 0x90 with 0, [`Turns::change_master`]), then the Enter
    /// ([`Turns::enter_with`]).
    ///
    /// # Errors
    ///
    /// What the master change and the Enter do not model.
    pub(crate) fn enter_order(
        &mut self,
        player: usize,
        idx: usize,
        order: crate::turn::EnterOrder,
    ) -> Result<(), GenieError> {
        if let Some(master) = order.switch {
            self.change_master(player, idx, master)?;
        }
        self.enter_with(player, idx, order.target, &order.task, order.under_artifact)
    }

    /// The enter command's Enter on `task` (`FUN_140a855d0`, [`Turns::new_enter`]): under an
    /// Artifact on top (`under_artifact`, read before the task search) it would become the
    /// Artifact's sub-action, which is not modelled; else the action list empties and the Enter
    /// is installed (slot 0x808, [`Turns::install_enter`]).
    ///
    /// # Errors
    ///
    /// What the Enter does not model.
    pub(crate) fn enter_with(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
        task: &Task,
        under_artifact: bool,
    ) -> Result<(), GenieError> {
        if under_artifact {
            return Err(ENTER_UNDER_ARTIFACT);
        }
        let (action, target, _) = self.new_enter(player, idx, target, task)?;
        self.install_enter(player, idx, target, action, task)
    }

    /// The Enter's constructor (`FUN_140a855d0`) on `target` with `task`: its target, an annex
    /// its main building; the point [`meeting`] gives at the target's height
    /// ([`Turns::enter_meeting`]); it takes the target (slot 0x178). With the target and its
    /// class.
    fn new_enter(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
        task: &Task,
    ) -> Result<(EnterAction, i32, i32), GenieError> {
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
            .ok_or(GenieError::unsupported("an Enter whose building is gone"))?;
        let (tp, t) = self.object(target).expect("found above");
        let tm = self.master_of(tp, t.life.master_id)?;
        let (class, height) = (i32::from(tm.class), t.position[2]);
        let (at, _) = self.enter_meeting(player, idx, target)?;
        let action = EnterAction::new(
            target,
            [at[0], at[1], height],
            task.work_range,
            task.object_group,
        );
        self.take(target, self.releaser(player, idx));
        Ok((action, target, class))
    }

    /// The Enter's install (slot 0x808): the action list empties and the Enter starts (slot 5,
    /// `FUN_140a86f20`): a unit inside an object (`set_state(13)`) and a target without room
    /// (`FUN_140a864d0`, `set_state(11)`) are not modelled; else its `set_state(4)`
    /// ([`EnterAction::walk_again`]): a unit in reach enters at once ([`Turns::enter_now`]),
    /// else it walks, a transport to the point first ([`Turns::move_transport`]).
    fn install_enter(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
        mut action: EnterAction,
        task: &Task,
    ) -> Result<(), GenieError> {
        let id = self.players[player].objects[idx].id;
        self.clear_actions(player, idx)?;
        let unit = &self.players[player].objects[idx];
        if unit.life.inside.is_some() {
            return Err(GenieError::unsupported(
                "an Enter of a unit inside an object (state 13)",
            ));
        }
        let (tp, t) = self.object(target).expect("found above");
        if !self.has_room(tp, t, Some(id))? {
            return Err(GenieError::unsupported(
                "an Enter into a building without room (state 11)",
            ));
        }
        let (t, (gap, speed), meeting) = self.entering(player, idx, target, task.work_range)?;
        let walking = action.walk_again(&t, gap, speed, &mut || meeting.clone())?;
        self.players[player].objects[idx].enter = Some(action);
        match walking {
            Walking::Enter => self.enter_now(player, idx, target),
            Walking::Walk(Some(to)) => self.move_transport(target, to),
            Walking::Walk(None) => Ok(()),
        }
    }

    /// What the Enter's start reads of unit `idx` of `player` and its target ([`enter_view`],
    /// room by slot 0x198).
    #[allow(clippy::type_complexity)]
    fn entering(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
        range: f32,
    ) -> Result<(EnterTarget, (f32, f32), Result<([f32; 2], u8), GenieError>), GenieError> {
        let id = self.players[player].objects[idx].id;
        let (tp, t) = self.object(target).ok_or(UNPLACED)?;
        let tm = self.master_of(tp, t.life.master_id)?.clone();
        let room = self.has_room(tp, t, Some(id))?;
        let t = match self.place(target) {
            Some(crate::order::Place::Live(p, i)) => &self.players[p].objects[i],
            Some(crate::order::Place::Sleeping(p, i)) => &self.players[p].sleeping[i],
            None => return Err(UNPLACED),
        };
        enter_view(
            &self.spatial,
            &mut self.zones,
            &self.masters,
            &self.players[player].objects[idx],
            (t, &tm, room),
            range,
        )
    }

    /// `FUN_140a85f30` ([`meeting`]) for unit `idx` of `player` and its target, as the
    /// constructor reads it ([`enter_view`]).
    fn enter_meeting(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
    ) -> Result<([f32; 2], u8), GenieError> {
        self.entering(player, idx, target, 0.0)?.2
    }

    /// The transport's part of the Enter's `set_state(4)` with code 1: its slot 0x3d8
    /// (`FUN_14114ffc0`) asks whether it reaches within 1 of the point's tile (the point
    /// truncated, [`crate::movement::Mover::reaches`]); then its slot 0x340 (`FUN_141216c50`:
    /// [`Turns::move_aside`], range 0, forced, running) moves it to the point at its height. One
    /// that cannot reach it is [`UNREACHED`].
    pub(crate) fn move_transport(
        &mut self,
        transport: i32,
        to: [f32; 3],
    ) -> Result<(), GenieError> {
        let Some(crate::order::Place::Live(tp, ti)) = self.place(transport) else {
            return Err(UNREACHED);
        };
        let ship = &self.players[tp].objects[ti];
        let mut plan = crate::movement::Planning {
            pathing: &mut self.pathing,
            now: self.world_time,
            computer: self.players[tp].computer,
            object: self.streams.get(crate::rng::Stream::Object),
            construction_rate: self.players[tp].construction_rate,
            groups: &self.groups,
        };
        if !ship_reaches(ship, &self.spatial, &mut plan, to)? {
            return Err(UNREACHED);
        }
        self.ship_move(transport, to)
    }

    /// The transport's slot 0x340 (`FUN_141216c50`, [`Turns::move_aside`]: range 0, forced,
    /// running) to `to`, where it meets an entering unit.
    pub(crate) fn ship_move(&mut self, ship: i32, to: [f32; 3]) -> Result<(), GenieError> {
        let Some(crate::order::Place::Live(p, i)) = self.place(ship) else {
            return Err(UNREACHED);
        };
        let walk = crate::unit_ai::AiMove {
            point: to,
            range: 0.0,
            force: true,
            walk: false,
        };
        self.move_aside(p, i, walk)
    }

    /// The enter at the Enter's start ([`crate::turn::Unit::enter_building`] into its target,
    /// any player's live object, then the start's own `set_state(1)`); the load it gives goes to
    /// the owner's attributes at once.
    fn enter_now(&mut self, player: usize, idx: usize, target: i32) -> Result<(), GenieError> {
        let inside = self
            .players
            .iter()
            .flat_map(|p| p.objects.iter().chain(&p.sleeping))
            .filter(|u| u.life.inside == Some(target))
            .count();
        let Some(crate::order::Place::Live(tp, ti)) = self.place(target) else {
            return Err(ENTER_SLEEPING);
        };
        let mut unit = std::mem::take(&mut self.players[player].objects[idx]);
        let mut adds = Vec::new();
        let result = (|| {
            let building = &self.players[tp].objects[ti];
            let owner = &self.players[player];
            let mut w = crate::turn::EnterWorld {
                spatial: &mut self.spatial,
                pathing: &mut self.pathing,
                masters: self.masters.get(player),
                target_masters: self.masters.get(tp),
                graphics: &self.graphics,
                sprite_rng: self.streams.get(crate::rng::Stream::Sprite),
                drop_sites: &self.drop_sites,
                attributes: &owner.attributes,
                civ: owner.civ,
                sharing: owner.sharing,
                player,
                attribute_adds: &mut adds,
                firepower: 0.0,
                left_group: None,
            };
            if !unit.enter_building(building, inside, &mut w)? {
                return Err(GenieError::unsupported(
                    "an Enter in reach that did not get in",
                ));
            }
            unit.entered(&mut w)?;
            Ok((w.firepower, w.left_group))
        })();
        self.players[player].objects[idx] = unit;
        let (firepower, left_group) = result?;
        let id = self.players[player].objects[idx].id;
        // its removeUnit from the unit group it left (FUN_14102c160), when the world holds it
        if let Some(g) = left_group.filter(|&g| self.groups.get(g).is_some()) {
            let _ = self.remove_unit(g, id, target, false)?;
        }
        let b = &mut self.players[tp].objects[ti];
        b.firepower += firepower;
        b.garrison.push(id);
        for (kind, amount) in adds {
            self.players[player].add_attribute(kind, amount)?;
        }
        Ok(())
    }

    /// The unit at `idx` of player `player` goes into `target` at once, its slot 0x1a0 forced
    /// (`FUN_141212e10` with 1: `FUN_1411921d0` passes over the room test, slot 0x198;
    /// [`crate::turn::Unit::go_inside`]): no load given, no notify, no state change. The
    /// building's list takes it and its volleys its share.
    pub(crate) fn enter_forced(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
    ) -> Result<(), GenieError> {
        let Some(crate::order::Place::Live(tp, ti)) = self.place(target) else {
            return Err(ENTER_SLEEPING);
        };
        let me = self
            .master_of(player, self.players[player].objects[idx].life.master_id)?
            .clone();
        let bm = self
            .master_of(tp, self.players[tp].objects[ti].life.master_id)?
            .clone();
        let mut unit = std::mem::take(&mut self.players[player].objects[idx]);
        let mut adds = Vec::new();
        let result: Result<_, GenieError> = (|| {
            let building = &self.players[tp].objects[ti];
            let owner = &self.players[player];
            let masters = self.masters.get(player).ok_or(NO_MASTERS)?;
            let mut w = crate::turn::EnterWorld {
                spatial: &mut self.spatial,
                pathing: &mut self.pathing,
                masters: Some(masters),
                target_masters: self.masters.get(tp),
                graphics: &self.graphics,
                sprite_rng: self.streams.get(crate::rng::Stream::Sprite),
                drop_sites: &self.drop_sites,
                attributes: &owner.attributes,
                civ: owner.civ,
                sharing: owner.sharing,
                player,
                attribute_adds: &mut adds,
                firepower: 0.0,
                left_group: None,
            };
            unit.go_inside(building, (masters, &me, &bm), &mut w)?;
            Ok((w.firepower, w.left_group))
        })();
        self.players[player].objects[idx] = unit;
        let (firepower, left_group) = result?;
        let id = self.players[player].objects[idx].id;
        if let Some(g) = left_group.filter(|&g| self.groups.get(g).is_some()) {
            let _ = self.remove_unit(g, id, target, false)?;
        }
        let b = &mut self.players[tp].objects[ti];
        b.firepower += firepower;
        b.garrison.push(id);
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
    ) -> Result<(), GenieError> {
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
                    return Err(GenieError::unsupported(
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
                return Err(GenieError::unsupported(
                    "an unload of a unit with units inside (a transport)",
                ));
            }
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
        }
        Ok(())
    }

    /// `GO_BACK_TO_WORK` (`do_command` 0x80 with one object, 0x86 with several:
    /// `FUN_141625590`) on the player's objects `ids` (another player's only while players may
    /// order each other's, [`crate::settings::Settings::cheats_enabled`]; the cheat `+0xb12` is not held): a building
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
    /// [`TownBell::back_to_work`]: crate::town_bell::TownBell::back_to_work
    pub(crate) fn back_to_work(
        &mut self,
        from: Option<(usize, usize)>,
        id: i32,
    ) -> Result<bool, GenieError> {
        use crate::town_bell::RecordTarget;
        let (up, ui) = match from {
            Some(_) => self.live_inside(id)?,
            None => match self.place(id) {
                Some(crate::order::Place::Live(p, i)) => (p, i),
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
    fn unload_picks(
        &self,
        player: usize,
        unit: &crate::turn::Unit,
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

    /// The player and live index of the unit `id` inside a building, whoever's it is.
    fn live_inside(&self, id: i32) -> Result<(usize, usize), GenieError> {
        match self.place(id) {
            Some(crate::order::Place::Live(p, i)) => Ok((p, i)),
            _ => Err(GenieError::unsupported(
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
    /// when no spot is free. With neither a point nor a gather point its unit AI stops, forgets
    /// its target and takes its stance again (slots 0xb0 and 0x1c0). Not modelled: the work given
    /// (slot 0x128 with a target), a gather point of kind 0xff,
    /// class 42's and livestock's counts in a building of a player with `+0x88`, a siege
    /// building's (class 13) slot 0x1b0; not held: the building's villager count (`+0xba8`) and
    /// raider count (`+0xc2c`, which the building's slot 0x1c8 `FUN_1411d6ad0` lowers for a unit
    /// of type 70 whose `+0x9a0` runs: such a unit inside is refused, [`Turns::garrisoned`]), its
    /// sound (slot 0x4c0), its garrison sprite. Its work command (slot 0x128) has `+0xa14` 0: the
    /// unit AI keeps its notifies and its queued orders.
    pub(crate) fn let_out(
        &mut self,
        player: usize,
        building: usize,
        id: i32,
        point: [f32; 2],
        (work_on, _sound): (Option<i32>, bool),
    ) -> Result<bool, GenieError> {
        let b = &self.players[player].objects[building];
        let bid = b.id;
        let bm = self.master_of(player, b.life.master_id)?.clone();
        let gather = b.gather_point;
        // the spot's goal (`FUN_1411d12d0` with x −1): without a gather point the first location
        // of the building unit AI's command queue (slot 0x218, then slot 0x220), else a gather
        // point that is not on the building, else none
        let queue_first =
            b.ai.as_ref()
                .and_then(|a| a.building_orders.first())
                .map(|o| [o.point[0], o.point[1]]);
        let toward = if point[0] == NO_POINT {
            match (gather, queue_first) {
                (None, Some(q)) => Some(q),
                _ => gather
                    .filter(|g| g.target != bid)
                    .map(|g| [g.point[0], g.point[1]]),
            }
        } else {
            Some(point)
        };
        // the unit is any player's (an ally's too): its own player's masters and orders
        let (up, ui) = self.live_inside(id)?;
        let unit = &self.players[up].objects[ui];
        let made = self.master_of(up, unit.life.master_id)?.clone();
        if bm.class == 13 {
            return Err(SIEGE_UNLOAD);
        }
        if made.class == WHOLE_SHARE_CLASS || made.class == 58 {
            return Err(GenieError::unsupported(
                "a class 42 or livestock unit leaving a building (its player's counts)",
            ));
        }
        if !unit.garrison.is_empty() {
            return Err(GenieError::unsupported(
                "a unit with units inside leaving a building",
            ));
        }
        let Ok(spot) = self.spot_beside(player, building, &made, toward) else {
            return Err(GenieError::unsupported(
                "an unload whose unit finds no spot (its later rings)",
            ));
        };
        let z = self.players[player].objects[building].position[2];
        let at = [spot[0], spot[1], z];
        // the building's slot 0x1c8 (`FUN_141217910`): a unit (type 70) takes its share off the
        // volleys (the building's rate by its player's masters, the unit's share by its own)
        let share = if made.kind == COMBAT_KIND {
            let (Some(own), Some(theirs)) = (self.masters.get(player), self.masters.get(up)) else {
                return Err(GenieError::unsupported("an unload without the masters"));
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
        let mut placed = self.spatial.come_out(id).ok_or(GenieError::unsupported(
            "a unit inside a building without its record",
        ))?;
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
            // neither a point nor a gather point: the unit AI stops its action (slot 0xb0 with
            // 1, whose stopAction runs on the unit), forgets its target (`FUN_140c5e1e0`) and
            // takes its own stance again at the unit's place (slot 0x1c0 with `+0x1a9`); a unit
            // without a unit AI stays where it came out
            let class = made.class;
            let unit = &mut self.players[up].objects[ui];
            let place = ([unit.position[0], unit.position[1]], class);
            if let Some(ai) = unit.ai.as_mut() {
                let mut fx = Vec::new();
                ai.stop_with(true, Some(place), &mut fx)?;
                ai.remove_target();
                let stance = ai.stance;
                crate::order::set_stance(ai, stance, place, &mut fx)?;
                for _ in fx.iter().filter(|e| **e == crate::unit_ai::Effect::Stop) {
                    self.command_stop(up, ui)?;
                }
            }
            return Ok(true);
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

    /// `UNIT_ORDER`'s garrison (action 5) on `target`, which the world holds (`FUN_140e7f200`; the
    /// UI's sound for it, `FUN_1409648c0`, is not held): `FUN_14161e370` lists the order's units
    /// (the ids the world holds of a type 70 or more), then the shelters ([`Shelters`]). Without
    /// `nearby` (the packet's `+0x1e`) the target alone: one of a type below 80 a transport,
    /// else a building (an annex its main building, but a gate's), with its count
    /// ([`Turns::garrisoned`]). With it, each object of the target's owner's tracker under the
    /// master the target's line stands for (`FUN_140fdd6c0`, `FUN_140fdd6f0`, [`UnitLines::base`];
    /// `FUN_14104f960`, [`crate::tracker::Tracker::list`]), in the tracker's order, within
    /// [`NEARBY`] of the target (`FUN_14118f8c0`, the squared footprint gap; `COMISS`/`JC`: a NaN
    /// is out): one of a type below 80 is a transport, one not a gate (class 39) and no annex
    /// (`+0xbb8`) a building, and each one in reach pushes its count, the gates and annexes
    /// left out too. Then [`Turns::shelter`] (`param_6` 0) with the unit AI's queue when
    /// `through_ai` (the packet's `+0x1d`).
    ///
    /// [`UnitLines::base`]: crate::lines::UnitLines::base
    pub(crate) fn garrison_order(
        &mut self,
        player: usize,
        ids: &[i32],
        target: i32,
        through_ai: bool,
        nearby: bool,
    ) -> Result<(), GenieError> {
        let Some((tp, t)) = self.object(target) else {
            return Ok(());
        };
        let (target_master, target_annex) = (t.life.master_id, t.annex_of);
        let mut units = Vec::with_capacity(ids.len());
        for &id in ids {
            let Some((p, u)) = self.object(id) else {
                continue;
            };
            if 70 <= self.master_of(p, u.life.master_id)?.kind {
                units.push(id);
            }
        }
        let shelters = if nearby {
            self.shelters_near(tp, target, target_master)?
        } else {
            let tm = self.master_of(tp, target_master)?;
            if tm.kind < BUILDING {
                Shelters {
                    buildings: Vec::new(),
                    transports: vec![target],
                    counts: vec![self.garrisoned(target)?],
                }
            } else {
                let building = match target_annex {
                    Some(main) if tm.class != GATE_CLASS => main,
                    _ => target,
                };
                Shelters {
                    buildings: vec![building],
                    transports: Vec::new(),
                    counts: vec![self.garrisoned(building)?],
                }
            }
        };
        self.shelter(player, &units, &shelters, false, through_ai)
    }

    /// `SEEK_SHELTER` (0x2c, `FUN_140e6a3a0`), after the command's check (`FUN_140e558b0`): of
    /// the first 60 `ids` (500 for a computer player), an id the world does not hold, and another
    /// player's unit unless players may order each other's ([`crate::settings::Settings::cheats_enabled`],
    /// `+0x1074`), are dropped ([`Turns::checked_units`]); with every id dropped the command does
    /// nothing. Of the others, the villagers (class 4 of type 70) and the units of master 1808
    /// seek shelter (a villager of master 1808 is listed twice) in the player's buildings that
    /// take villagers: in the player's object order, those of type 80 in state 2, not inside
    /// another (`+0x2f0`, `FUN_141188da0`), whose master's `garrison_type` has bit 0
    /// (`FUN_1411ccb60` with 1), each with its count ([`Turns::garrisoned`]); then
    /// [`Turns::shelter`] (`FUN_141622450`).
    pub(crate) fn seek_shelter(&mut self, player: usize, ids: &[i32]) -> Result<(), GenieError> {
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
        let counts = buildings
            .iter()
            .map(|&b| self.garrisoned(b))
            .collect::<Result<Vec<_>, GenieError>>()?;
        let shelters = Shelters {
            buildings,
            transports: Vec::new(),
            counts,
        };
        // FUN_141622450: no transports, no gap limit, not through the unit AIs
        self.shelter(player, &units, &shelters, false, false)
    }

    /// `FUN_14161e370`'s shelters near `target` (of player `tp` and master `master`, its
    /// `param_9`): the objects of `tp`'s tracker under the master `master`'s line stands for, in
    /// the tracker's order ([`Turns::garrison_order`]).
    pub(crate) fn shelters_near(
        &self,
        tp: usize,
        target: i32,
        master: i16,
    ) -> Result<Shelters, GenieError> {
        let mut shelters = Shelters::default();
        let key = self.lines.base(i32::from(master));
        let (at, radius) = self.ground(target)?;
        for &id in self.players[tp].tracker.list(key) {
            // the tracker's keepers follow every object its player gains and loses, so the
            // world holds each (the game reads a null one's place)
            let Some((op, o)) = self.object(id) else {
                continue;
            };
            let (o_at, o_radius) = self.ground(id)?;
            // COMISS NEARBY, gap; JC: a gap above it, or a NaN, is out
            let gap = footprint_gap_squared(o_at, o_radius, at, radius);
            if crate::fmath::below_or_unordered(NEARBY, gap) {
                continue;
            }
            let om = self.master_of(op, o.life.master_id)?;
            if om.kind < BUILDING {
                shelters.transports.push(id);
            } else if om.class != GATE_CLASS && o.annex_of.is_none() {
                shelters.buildings.push(id);
            }
            shelters.counts.push(self.garrisoned(id)?);
        }
        Ok(shelters)
    }

    /// The place and master radii of the object `id` (what `FUN_14118f8c0` reads of it).
    pub(crate) fn ground(&self, id: i32) -> Result<([f32; 3], [f32; 2]), GenieError> {
        self.spatial
            .get_known(id)?
            .map(|p| (p.position, p.radius))
            .ok_or(GenieError::unsupported(
                "a garrison's object without a place",
            ))
    }

    /// `FUN_141621220`: each of `units` pairs with each building of `shelters` (type 80) whose
    /// master has a capacity (`obj_max`) and takes the unit's master's copy in
    /// ([`Turns::takes_in`]), and with each transport of a unit whose master's class has a flag
    /// of 0x10002 ([`class_has_flags`]) when the commanding player's master of the unit's master
    /// id has a task of action 3 on the transport's class ([`has_garrison_task`]), within the reach
    /// (`near`, `param_6`: [`TOWN_BELL_REACH`], else [`FAR`]; `COMISS`/`JBE`: a NaN is out); a
    /// pair names its shelter by its index, a transport's after the buildings. The pairs sorted
    /// by their squared footprint gap (`FUN_1415f0830`, MSVC's `std::sort`) give each unit, in
    /// that order, the first shelter whose count is below its master's capacity, then any unit
    /// left its nearest shelter. The count is the one of the count list at the pair's index
    /// (the list holds what its caller pushed, in its own order: an index into it need not be
    /// its shelter's), which each taker raises. The shelters go in a hash map by id
    /// (`FUN_1415f0c40`), whose order ([`crate::msvc::unordered_map_order`]) the next part
    /// follows ([`Turns::shelter_in`]). Not modelled: the groups the splitter forms.
    pub(crate) fn shelter(
        &mut self,
        player: usize,
        units: &[i32],
        shelters: &Shelters,
        near: bool,
        through_ai: bool,
    ) -> Result<(), GenieError> {
        let reach = if near { TOWN_BELL_REACH } else { FAR };
        let all: Vec<i32> = shelters
            .buildings
            .iter()
            .chain(&shelters.transports)
            .copied()
            .collect();
        // the pairs of a unit and a shelter: (unit index, shelter index, squared gap)
        let mut pairs = Vec::new();
        let mut capacities = Vec::with_capacity(all.len());
        for &building in &all {
            let (bp, b) = self
                .object(building)
                .ok_or(GenieError::unsupported("a garrison's building is gone"))?;
            let bm = self.master_of(bp, b.life.master_id)?;
            capacities.push((bm.kind == BUILDING, i32::from(bm.garrison), bm.class));
        }
        let first_transport = shelters.buildings.len();
        for (i, &id) in units.iter().enumerate() {
            let (p, u) = self
                .object(id)
                .ok_or(GenieError::unsupported("a garrison's unit is gone"))?;
            let own = self.master_of(p, u.life.master_id)?;
            let (copy, master, class) = (own.copy_id, own.id, own.class);
            // COMISS reach, gap; JBE: a gap at the reach or past it, or a NaN, pairs nothing
            let gap_to = |shelter: i32| -> Result<f32, GenieError> {
                let (at, r) = self.ground(id)?;
                let (sat, sr) = self.ground(shelter)?;
                Ok(footprint_gap_squared(at, r, sat, sr))
            };
            for (j, &building) in shelters.buildings.iter().enumerate() {
                let (is_building, capacity, _) = capacities[j];
                if is_building && capacity != 0 && self.takes_in(building, player, copy)? {
                    let gap = gap_to(building)?;
                    if gap < reach {
                        pairs.push((i, j, gap));
                    }
                }
            }
            if !class_has_flags(class, TRANSPORTED) {
                continue;
            }
            for (k, &transport) in shelters.transports.iter().enumerate() {
                // FUN_1415c9240 reads the commanding player's master of the unit's master id
                // (its slot unchecked: one the player lacks would crash the game)
                let (_, _, transport_class) = capacities[first_transport + k];
                let list = self.master_of(player, master)?;
                if has_garrison_task(&list.tasks, GARRISON, transport_class) {
                    let gap = gap_to(transport)?;
                    if gap < reach {
                        pairs.push((i, first_transport + k, gap));
                    }
                }
            }
        }
        crate::msvc::sort(
            &mut pairs,
            &|a: (usize, usize, f32), b: (usize, usize, f32)| a.2 < b.2,
        );
        let mut counts = shelters.counts.clone();
        let mut taken = vec![false; units.len()];
        // each shelter's units, by shelter index, and the order of the shelters' first take
        let mut going: Vec<Vec<i32>> = vec![Vec::new(); all.len()];
        let mut first_taken = Vec::new();
        for &(i, j, _) in &pairs {
            // the count list is at least as long as the shelters: its callers push a count for
            // each shelter, and FUN_14161e370 one for each gate and annex beside
            let count = counts.get_mut(j).expect("a count for each shelter index");
            if !taken[i] && *count < capacities[j].1 {
                going[j].push(units[i]);
                first_taken.push(all[j]);
                *count += 1;
                taken[i] = true;
            }
        }
        for &(i, j, _) in &pairs {
            if !taken[i] {
                going[j].push(units[i]);
                first_taken.push(all[j]);
                taken[i] = true;
            }
        }
        for building in crate::msvc::unordered_map_order(first_taken) {
            let j = all
                .iter()
                .position(|&b| b == building)
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
    ) -> Result<bool, GenieError> {
        if self.master_of(bp, b.life.master_id)?.kind < COMBAT_KIND {
            return Ok(false);
        }
        self.takes_in(b.id, player, master)
    }

    /// One shelter's part of [`Turns::shelter`]: the order splitter (`FUN_140fd9dc0` with
    /// command `0x2713`) on its units, which keeps those whose unit AI's test passes
    /// ([`Turns::ai_takes`]); then each loose unit (not in a group, `+0x6d0`; in state 2 or 6 or
    /// less, of a type 70 or more) takes slot 0x7e8 on the shelter at its place with task
    /// filter 3, or, `through_ai` (`param_7`) and with a unit AI, the unit AI's slot 0x30 (into
    /// slot 0x28, `FUN_140c55c90`: not immediate, behind) queues the enter order (`0x2cd`) on
    /// the shelter at its place: issued by the unit's own id, of the shelter's owner and master,
    /// range 0, priority 100, the point shown, ready.
    fn shelter_in(
        &mut self,
        building: i32,
        going: &[i32],
        through_ai: bool,
    ) -> Result<(), GenieError> {
        let (bp, b) = self
            .object(building)
            .ok_or(GenieError::unsupported("a garrison's building is gone"))?;
        // the splitter tests a unit with a unit AI only
        let keeps = |id: i32| -> Result<bool, GenieError> {
            let (p, u) = self.object(id).expect("a going unit");
            if u.ai.is_none() {
                return Ok(true);
            }
            self.ai_takes(bp, b, p, u.life.master_id)
        };
        let (point, owner, master) = (b.position, bp, b.life.master_id);
        let mut kept = Vec::with_capacity(going.len());
        for &id in going {
            if self.object(id).is_some() && keeps(id)? {
                kept.push(id);
            }
        }
        let loose = self.loose_units_kept(going, &|id| kept.contains(&id))?;
        for id in loose {
            let Some(crate::order::Place::Live(p, idx)) = self.place(id) else {
                return Err(GenieError::unsupported(
                    "a garrison of a unit the slice does not hold live",
                ));
            };
            let unit = &self.players[p].objects[idx];
            let kind = self.master_of(p, unit.life.master_id)?.kind;
            if (2 < unit.life.state && unit.life.state != 6) || kind < 70 {
                continue;
            }
            if through_ai && unit.ai.is_some() {
                let state = unit.life.state;
                let order = crate::unit_ai::QueuedOrder {
                    issuer: id,
                    order: ENTER_ORDER,
                    priority: 100,
                    target: building,
                    target_owner: owner as i32,
                    point,
                    range: 0.0,
                    shown: [point[0], point[1]],
                    ready: true,
                    target_type: i32::from(master),
                };
                let ai = self.players[p].objects[idx]
                    .ai
                    .as_mut()
                    .expect("tested above");
                let _ = ai.queue_order(state, order, false, false)?;
                continue;
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
/// `set_state(2)`'s wait before the Enter walks again, in seconds (`FUN_140a87320`, the constant
/// `0x3f800000` it stores).
const WAIT_SECONDS: f32 = 1.0;
/// The class of siege weapons, whose own place an Enter's point keeps (`FUN_140a85f30`, the
/// short `0xd`).
const SIEGE_CLASS: i16 = 13;

/// An unload of a siege building or tower (class 13: its spot search `FUN_141192d10` and the
/// unit's leave), not modelled.
pub(crate) const SIEGE_UNLOAD: GenieError =
    GenieError::unsupported("an unload of a siege building (class 13)");

/// A unit's Enter action (`RGE_Action_Enter`, type 3): it walks to a building or a transport and
/// enters it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EnterAction {
    /// The building or transport (`+0x20`, its id `+0x30`).
    pub target: i32,
    /// The action state (`+0x18`).
    pub state: u8,
    /// Its `MoveTo` sub-action's state.
    pub move_to: Option<MoveTo>,
    /// `+0x38`..`+0x40`: where it meets the target ([`meeting`]: a building's place; for a
    /// transport where the two meet, its height the target's at the constructor), which a
    /// target that moves leaves.
    pub point: [f32; 3],
    /// The task's work range (`+0x30`): a building's `MoveTo` stops this near, and the enter
    /// needs it.
    pub range: f32,
    /// `+0x44`: state 2's wait, in seconds.
    pub timer: f32,
    /// `+0x68`, 1 at first: a `MoveTo` that fails (code 3) asks once more.
    pub retry: bool,
    /// `+0x6c`: the task's class (its `+0x10`).
    pub task_class: i16,
    /// Its `MoveTo`'s wait after the motion step gave up (`+0x6c`..`+0x74`).
    pub move_wait: crate::move_to::MoveWait,
}

/// What an Enter's update reads of its target.
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
    /// It is a building (type 80); else a transport.
    pub building: bool,
}

/// What an Enter's update leaves to its caller.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EnterStep {
    /// The update returned 1 (state 1): the action list removes it.
    pub done: bool,
    /// The unit reached the target: the caller enters it (`FUN_140a867b0`, whose end is the
    /// action's `set_state(1)`), then the caller's own `set_state(1)`.
    pub enter: bool,
    /// The enter came from the `set_state(4)` of a target that moved, in state 4: the update then
    /// finds no `MoveTo` (code 1) with the unit in reach and enters again (`FUN_140a867b0` on a
    /// unit inside it already: notify 0x1fa and `set_state(1)`), then its own `set_state(1)`.
    pub again: bool,
    /// `set_state(2)`: the unit shows its master's standing sprite (slot 0x68 with `+0x470`).
    pub waits: bool,
    /// `set_state(4)` found the transport where it meets the unit (its [`meeting`] code 1): the
    /// caller asks the transport to move there (slot 0x3d8 within 1, then slot 0x340), the
    /// point's height the transport's.
    pub order: Option<[f32; 3]>,
}

/// What [`EnterAction::walk_again`] (`set_state(4)`) leaves to its caller.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Walking {
    /// The unit is in reach: the caller enters (`FUN_140a867b0`), then `set_state(1)`.
    Enter,
    /// A `MoveTo` to the target started; with a point, the transport is to move there first
    /// ([`EnterStep::order`]).
    Walk(Option<[f32; 3]>),
}

impl EnterAction {
    /// The factory's constructor (`FUN_140a855d0`): the target (an annex's main building, the
    /// caller's, as its add, slot 0x178), the point [`meeting`] gives at the target's height,
    /// the task's range and class, `+0x68` 1; the start (slot 5, `FUN_140a86f20`) and its
    /// `set_state(4)` are [`EnterAction::walk_again`].
    #[must_use]
    pub const fn new(target: i32, point: [f32; 3], range: f32, task_class: i16) -> Self {
        Self {
            target,
            state: WALK,
            move_to: None,
            point,
            range,
            timer: 0.0,
            retry: true,
            task_class,
            move_wait: crate::move_to::MoveWait {
                state: 0,
                timer: 0.0,
                delay: 0.0,
            },
        }
    }

    /// `set_state(4)` (`FUN_140a87320` case 4) after the start's or the update's tests: its
    /// sub-actions go (`FUN_140a98620`; a `MoveTo`'s teardown stops the unit, the caller's), and
    /// its room test repeats theirs, which nothing between changes. A unit in reach of the
    /// target (`FUN_140a85c30`) enters it ([`Walking::Enter`]). Else a building's `MoveTo` walks
    /// to it within the task's range (`FUN_140aa0df0`); a transport's takes the point and code
    /// of [`meeting`] (`FUN_140a85f30`, the action's `+0x38` and `+0x3c`) and walks to the
    /// transport within 0 (`FUN_140aa0d30` with range 0, then slot 0x68 with the transport: its
    /// state 3 goes to the transport's place; a point below 0 would make `FUN_140aa0df0` with the
    /// unit's slot 0x268, 0, alike), and code 1 asks the transport to move to the point. Both
    /// `MoveTo`s are quiet (`FUN_140a7a170` with 1) and leave a target that moves (`+0x61` 2);
    /// their start ([`EnterAction::walk_start`]) is `0x140aa2100`'s, by the footprints' gap
    /// `gap` and the unit's speed.
    pub fn walk_again(
        &mut self,
        t: &EnterTarget,
        gap: f32,
        speed: f32,
        meeting: &mut impl FnMut() -> Result<([f32; 2], u8), GenieError>,
    ) -> Result<Walking, GenieError> {
        self.move_to = None;
        self.state = WALK;
        if t.in_reach {
            return Ok(Walking::Enter);
        }
        let code = if t.building {
            0
        } else {
            let (point, code) = meeting()?;
            self.point[..2].copy_from_slice(&point);
            code
        };
        self.move_to = Some(Self::walk_start(gap, speed)?);
        Ok(Walking::Walk((code == 1).then_some([
            self.point[0],
            self.point[1],
            t.position[2],
        ])))
    }

    /// The `MoveTo`'s start (`0x140aa2100` with 1): beyond [`ARRIVED`] of the target's footprint
    /// (`gap`) state 3, or 13 for a unit without speed (not modelled); else state 1.
    fn walk_start(gap: f32, speed: f32) -> Result<MoveTo, GenieError> {
        if ARRIVED < gap {
            if speed == 0.0 {
                return Err(GenieError::unsupported(
                    "an Enter's MoveTo of a unit without speed",
                ));
            }
            return Ok(MoveTo::Start);
        }
        Ok(MoveTo::Arrived)
    }

    /// The range its `MoveTo` stops within: the task's for a building, 0 for a transport.
    const fn walk_range(&self, building: bool) -> f32 {
        if building { self.range } else { 0.0 }
    }

    /// `set_state(2)`: the wait of [`WAIT_SECONDS`] (the standing sprite is the caller's,
    /// [`EnterStep::waits`]).
    const fn wait(&mut self, step: &mut EnterStep) {
        self.move_to = None;
        self.state = WAIT;
        self.timer = WAIT_SECONDS;
        step.waits = true;
    }

    /// `FUN_140a85c30`: the unit is in reach of the target: their footprints' squared gap
    /// (`FUN_14118f8c0`; `COMISS`/`JBE`: a NaN gap is not out) at most the range squared; for a
    /// range of at most 1 (`COMISS`/`JC`: a NaN range is in reach) to a target with speed (slot
    /// 0x208, above 0) the tiles about the two ([`walled_off`], `tiles`) must not hold exactly
    /// two buildings.
    pub fn in_reach(
        range: f32,
        gap_squared: f32,
        target_speed: f32,
        tiles: impl FnOnce() -> Result<bool, GenieError>,
    ) -> Result<bool, GenieError> {
        if range * range < gap_squared {
            return Ok(false);
        }
        if range <= ENTER_NEAR && 0.0 < target_speed {
            return Ok(!tiles()?);
        }
        Ok(true)
    }

    /// One update (slot 6, `FUN_140a87a80`). It first checks its target: one gone, dead (state 3
    /// or more) or of an unallied player, or without room for the unit, makes the action let it
    /// go and turn to state 11, which is not modelled. State 1 is done; state 2 waits its timer
    /// (by the world's `dt`), then walks again ([`EnterAction::walk_again`]). State 4: a target
    /// off the action's point (`FUN_140a7bbf0`: any of x, y and height not equal) by more than
    /// [`HALF_TILE`] on an axis (`FUN_140a7bd10`: `COMISS`/`JA`, a NaN is not) walks it again; then
    /// the `MoveTo` updates (`FUN_140a992b0`: code 1 once it ended, or without it): codes 1 and
    /// 2 enter in reach ([`EnterStep::enter`]), else wait (`set_state(2)`); code 3 (a `MoveTo`
    /// that failed) walks again once (`+0x68`), then waits; codes 4 and 5 let the target go
    /// (state 11), which is not modelled. State 13 returns 3, which is not modelled. `gap` and
    /// `speed` start a new `MoveTo` ([`EnterAction::walk_start`]); `meeting` gives a transport's
    /// point ([`meeting`]) and `reaches` its reach test there ([`EnterAction::again`]).
    pub fn update(
        &mut self,
        mut walk: Walk<'_>,
        target: Option<EnterTarget>,
        (gap, speed): (f32, f32),
        mut meeting: impl FnMut() -> Result<([f32; 2], u8), GenieError>,
        mut reaches: impl FnMut(&mut Walk<'_>, [f32; 3]) -> Result<bool, GenieError>,
        (waits, stance): (bool, u8),
    ) -> Result<EnterStep, GenieError> {
        let mut step = EnterStep::default();
        let Some(t) = target else {
            return Err(GenieError::unsupported(
                "an Enter whose building is gone (state 11)",
            ));
        };
        if 3 <= t.state || !t.allied {
            return Err(GenieError::unsupported(
                "an Enter whose building died or is not allied (state 11)",
            ));
        }
        if !t.room {
            return Err(GenieError::unsupported(
                "an Enter whose building has no room (state 11)",
            ));
        }
        match self.state {
            ENTERED => step.done = true,
            WAIT => {
                self.timer -= walk.dt;
                if self.timer <= 0.0 {
                    self.again(
                        &mut walk,
                        &t,
                        (gap, speed),
                        &mut meeting,
                        &mut reaches,
                        &mut step,
                    )?;
                }
            }
            WALK => {
                let [x, y, z] = t.position;
                let moved = |d: f32| HALF_TILE < d.abs();
                let walked_again = t.position != self.point
                    && (moved(x - self.point[0])
                        || moved(y - self.point[1])
                        || moved(z - self.point[2]));
                if walked_again {
                    self.again(
                        &mut walk,
                        &t,
                        (gap, speed),
                        &mut meeting,
                        &mut reaches,
                        &mut step,
                    )?;
                }
                let range = self.walk_range(t.building);
                let code = match self.move_to {
                    None | Some(MoveTo::Arrived) => 1,
                    // the motion step giving up makes it wait or fail ([`MoveWait::give_up`])
                    Some(MoveTo::Start) => {
                        if self.move_wait.state == 1 {
                            self.move_wait.state = 2;
                        }
                        self.move_to = Some(MoveTo::Moving);
                        self.move_to = Some(
                            match start_walk_or_give_up(
                                walk.reborrow(),
                                Goal::Target(self.target),
                                range,
                                true,
                            )? {
                                Some(state) => state,
                                None => self.move_wait.give_up(waits, stance),
                            },
                        );
                        0
                    }
                    Some(MoveTo::Moving) => {
                        self.move_to = Some(match walk_or_give_up(walk.reborrow())? {
                            Some(state) => state,
                            None => self.move_wait.give_up(waits, stance),
                        });
                        0
                    }
                    Some(MoveTo::Failed) => 3,
                    // its wait: the world's dt onto its timer; at its delay it asks again
                    // (set_state(3); COMISS, JC: a NaN timer waits on); its update returns 0
                    Some(MoveTo::Waiting) => {
                        self.move_wait.timer += walk.dt;
                        if self.move_wait.delay <= self.move_wait.timer {
                            self.move_wait.timer = 0.0;
                            self.move_to = Some(MoveTo::Start);
                        }
                        0
                    }
                };
                match code {
                    // the walk again's enter put the unit inside, at the target's place: still
                    // in reach, it enters again
                    1 | 2 if walked_again && step.enter => step.again = true,
                    1 | 2 if t.in_reach => step.enter = true,
                    // a failed MoveTo is none a walk again made in this update
                    3 if self.retry => {
                        self.retry = false;
                        self.again(
                            &mut walk,
                            &t,
                            (gap, speed),
                            &mut meeting,
                            &mut reaches,
                            &mut step,
                        )?;
                    }
                    1..=3 => self.wait(&mut step),
                    _ => {}
                }
            }
            _ => return Err(GenieError::unsupported("an Enter state not modelled")),
        }
        Ok(step)
    }

    /// The update's `set_state(4)` ([`EnterAction::walk_again`]): its `MoveTo`'s teardown stops
    /// the unit (slot 0x5d0); an enter is the caller's. A transport's order first tests that the
    /// transport reaches within 1 of the point's tile (slot 0x3d8, `FUN_14114ffc0` with the
    /// point truncated: `reaches`); the move itself (slot 0x340) is the caller's
    /// ([`EnterStep::order`]). A transport that cannot reach it makes `set_state(13)`, which is
    /// not modelled.
    fn again(
        &mut self,
        walk: &mut Walk<'_>,
        t: &EnterTarget,
        (gap, speed): (f32, f32),
        meeting: &mut impl FnMut() -> Result<([f32; 2], u8), GenieError>,
        reaches: &mut impl FnMut(&mut Walk<'_>, [f32; 3]) -> Result<bool, GenieError>,
        step: &mut EnterStep,
    ) -> Result<(), GenieError> {
        if self.move_to.is_some()
            && let Some(m) = walk.mover.as_deref_mut()
        {
            m.stop()?;
        }
        match self.walk_again(t, gap, speed, meeting)? {
            Walking::Enter => step.enter = true,
            Walking::Walk(Some(at)) => {
                if !reaches(walk, at)? {
                    return Err(UNREACHED);
                }
                step.order = Some(at);
            }
            Walking::Walk(None) => {}
        }
        Ok(())
    }
}

/// How near the point a transport must reach for an Enter (slot 0x3d8's range), and the range up
/// to which an Enter's reach of a target with speed tests the tiles about the two
/// (`FUN_140a85c30`): `DAT_14330eb8c`, 1.0.
const ENTER_NEAR: f32 = crate::image::garrison::ENTER_NEAR;
/// Half a tile (`DAT_142e7f384`, 0.5): what an Enter's meeting point adds to a tile or to a
/// place before `floorf` (`FUN_140a85f30`), and how far a target leaves its point before the
/// Enter walks again (`FUN_140a7bd10`).
const HALF_TILE: f32 = crate::image::garrison::HALF_TILE;

/// An Enter whose transport cannot reach where it meets the unit (`set_state(13)` in
/// `set_state(4)`), which is not modelled.
pub(crate) const UNREACHED: GenieError =
    GenieError::unsupported("an Enter's transport that cannot reach the unit (state 13)");

/// What [`meeting`] (`FUN_140a85f30`) reads of an Enter's target.
#[derive(Clone, Debug)]
pub struct MeetingTarget {
    /// It has room for the unit (slot 0x198 with the unit).
    pub room: bool,
    /// Its speed (slot 0x208).
    pub speed: f32,
    /// Its top action's state (slot 0x2b0, `RGE_Action_Object::actionState`; 0 without one).
    pub state: u8,
    /// Its class (`+0x410`).
    pub class: i16,
    /// Its place, x and y.
    pub position: [f32; 2],
    /// Its top action's point (`FUN_140a7be40`, `FUN_140a7be50`), or why the slice has not got it.
    pub action_point: Result<[f32; 2], GenieError>,
}

/// `FUN_140a85f30`: where an Enter meets its target and the code its `set_state(4)` reads (1:
/// the transport moves there). A target without room for the unit (slot 0x198), or without speed
/// (`COMISS 0, speed` with `JNC`: a NaN speed has some), is met at its place. One idle (its
/// action state 0) or in state 2 and with room meets the unit where its slot 0x428
/// ([`landing_tile`], `landing`) finds the shore, at the tile's centre (code 1), else, the unit
/// standing on a beach (`beach`, [`crate::terrain::Terrain::beach`]), at the unit's place rounded
/// to the nearest corner (`floorf` of each plus 0.5, code 1), else at its place. One under an
/// action in state 3, 4 or 11 meets it where that action goes, but a siege weapon (class 13)
/// at its place; any other state at its place.
pub fn meeting(
    t: &MeetingTarget,
    unit: [f32; 2],
    landing: impl FnOnce() -> Result<Option<[i32; 2]>, GenieError>,
    beach: impl FnOnce() -> bool,
) -> Result<([f32; 2], u8), GenieError> {
    let at = (t.position, 0);
    if !t.room || t.speed <= 0.0 {
        return Ok(at);
    }
    if matches!(t.state, 0 | 2) {
        if let Some([x, y]) = landing()? {
            return Ok(([x as f32 + HALF_TILE, y as f32 + HALF_TILE], 1));
        }
        // the state is 0 or 2 here, never 11, 4 or 3
        if beach() {
            return Ok((
                [(unit[0] + HALF_TILE).floor(), (unit[1] + HALF_TILE).floor()],
                1,
            ));
        }
        return Ok(at);
    }
    if t.class == SIEGE_CLASS || !matches!(t.state, 11 | 4 | 3) {
        return Ok(at);
    }
    Ok((t.action_point.clone()?, 0))
}

/// `FUN_141157840` (a unit's slot 0x428, its last argument 1): from tile `from` (a transport's)
/// toward tile `to` (the unit's) in twice as many steps as the longer axis's tiles (the `y`
/// axis only when it is strictly longer), each step adding the whole difference over the step
/// count in floats, the first new tile on the way that `lands` (its tile and the float place
/// reached: a beach in the unit's zone where the unit could stand, slot 0x278) is where the two
/// meet. None when the tiles are one, or no tile lands.
pub fn landing_tile(
    from: [i32; 2],
    to: [i32; 2],
    mut lands: impl FnMut([i32; 2], [f32; 2]) -> Result<bool, GenieError>,
) -> Result<Option<[i32; 2]>, GenieError> {
    let (dx, dy) = (to[0].wrapping_sub(from[0]), to[1].wrapping_sub(from[1]));
    let (fx, fy) = (dx as f32, dy as f32);
    if fx == 0.0 && fy == 0.0 {
        return Ok(None);
    }
    let longer = if fy.abs() <= fx.abs() { dx } else { dy };
    let steps = cvttss2si(longer as f32).wrapping_abs().wrapping_mul(2);
    let (mut x, mut y) = (from[0] as f32, from[1] as f32);
    let mut last = [-999, -999];
    for _ in 0..steps.max(0) {
        x += fx / steps as f32;
        y += fy / steps as f32;
        let tile = [cvttss2si(x), cvttss2si(y)];
        if tile != last {
            last = tile;
            if lands(tile, [x, y])? {
                return Ok(Some(tile));
            }
        }
    }
    Ok(None)
}

/// `FUN_140a85c30`'s tile test for a range of at most 1 to a target with speed: about the tiles
/// of the unit's and the target's movement places (slot 0x520, `+0x824`; `me` and `them`),
/// their box (when it spans more than one tile on an axis, its first tile and the next, within
/// the map `map`), each tile but those two where the unit could not stand (`stands`, slot 0x278
/// at the tile's corner) counts the buildings on it (`buildings`, type 80 of the tile's list).
/// True when they are exactly two: the unit is not in reach.
pub fn walled_off(
    me: [f32; 2],
    them: [f32; 2],
    map: [i32; 2],
    mut stands: impl FnMut(i32, i32) -> Result<bool, GenieError>,
    mut buildings: impl FnMut(i32, i32) -> Result<usize, GenieError>,
) -> Result<bool, GenieError> {
    let (mx, my) = (cvttss2si(me[0]), cvttss2si(me[1]));
    let (tx, ty) = (cvttss2si(them[0]), cvttss2si(them[1]));
    let (x0, y0) = (mx.min(tx), my.min(ty));
    let (mut x1, mut y1) = (mx.max(tx), my.max(ty));
    if 1 < x1.wrapping_sub(x0) || 1 < y1.wrapping_sub(y0) {
        x1 = (x0 + 1).min(map[0] - 1);
        y1 = (y0 + 1).min(map[1] - 1);
    }
    let mut count = 0usize;
    for x in x0..=x1 {
        for y in y0..=y1 {
            if (x, y) == (mx, my) || (x, y) == (tx, ty) || stands(x, y)? {
                continue;
            }
            count += buildings(x, y)?;
        }
    }
    Ok(count == 2)
}

/// [`MeetingTarget`] of the target `t` (of master `tm`) with `room` for the unit: its top speed
/// ([`crate::movement::MoveStats::speed`]), its top action's state and, for a `MoveTo`, its
/// point (its goal, or for one on an object the mover's [`crate::movement::Mover::move_to_point`]).
/// An action the slice does not hold has neither; one but a `MoveTo` no point the slice keeps.
pub(crate) fn meeting_target(
    t: &crate::turn::Unit,
    tm: &Master,
    room: bool,
) -> Result<MeetingTarget, GenieError> {
    if let Some(e) = t.unmodelled_action.clone() {
        return Err(e);
    }
    let action_point = match (t.top_action_kind(), t.move_to) {
        (Some(crate::move_to::MOVE_TO), Some(m)) => m
            .goal
            .or_else(|| t.mover.as_ref().and_then(|v| v.move_to_point))
            .map(|p| [p[0], p[1]])
            .ok_or(UNPLACED),
        _ => Err(UNPLACED),
    };
    Ok(MeetingTarget {
        room,
        speed: t.mover.as_ref().map_or(0.0, |m| m.stats.speed),
        state: t.top_action().map_or(0, |a| a.state),
        class: tm.class,
        position: [t.position[0], t.position[1]],
        action_point,
    })
}

/// A transport's top action whose point the slice does not keep (`FUN_140a7be40`).
const UNPLACED: GenieError =
    GenieError::unsupported("an Enter's transport under an action the slice does not place");

/// [`meeting`] for `unit` (with its movement state `stats`) and its target `t`: the shore on the
/// way from the target's tile to the unit's ([`landing_tile`]): a beach
/// ([`crate::terrain::Terrain::beach`]) whose zone for the unit's restriction (`FUN_1411958c0`:
/// 0 for a tile below 0) is the zone of the unit's own tile (`FUN_14118eea0`), where the unit
/// could stand (slot 0x278, [`crate::spatial::Spatial::point_clear`] with the master's x radius
/// and the restriction's mask); else a beach under the unit (`FUN_1413fbf30`).
pub(crate) fn meeting_point(
    t: &MeetingTarget,
    unit: (i32, [f32; 3], &crate::movement::MoveStats),
    spatial: &crate::spatial::Spatial,
    zones: &mut crate::zones::Zones,
) -> Result<([f32; 2], u8), GenieError> {
    let (id, [ux, uy, _], stats) = unit;
    let terrain = spatial.terrain.as_ref();
    let landing = || {
        let terrain = terrain.ok_or(GenieError::unsupported(
            "the map's zones without its terrain",
        ))?;
        let values = terrain
            .restrictions
            .get(stats.restriction)
            .ok_or(GenieError::unsupported(
                "a restriction the slice's data has not got",
            ))?;
        let [w, h] = spatial.map;
        let map = zones.of(w, h, &terrain.kinds, values);
        let own = map.zone(cvttss2si(ux), cvttss2si(uy));
        let from = [cvttss2si(t.position[0]), cvttss2si(t.position[1])];
        landing_tile(from, [cvttss2si(ux), cvttss2si(uy)], |[x, y], [fx, fy]| {
            if !terrain.beach(x, y) {
                return Ok(false);
            }
            let zone = if x < 0 || y < 0 { 0 } else { map.zone(x, y) };
            if zone != own {
                return Ok(false);
            }
            spatial.point_clear(id, [fx, fy], stats.radius[0], stats.restriction_mask)
        })
    };
    let beach = || terrain.is_some_and(|tr| tr.beach(cvttss2si(ux), cvttss2si(uy)));
    meeting(t, [ux, uy], landing, beach)
}

/// [`EnterAction::in_reach`] of the unit `id` (its movement state `stats`, its place in
/// `spatial`) for its target `target` (its top speed and where it last moved to): the squared
/// footprints' gap of their places, and the tile test ([`walled_off`]) about where each last
/// moved to (slot 0x520), its buildings those whose master (`masters`, each player's) is of
/// type 80.
pub(crate) fn enter_reach(
    spatial: &crate::spatial::Spatial,
    masters: &[Masters],
    (id, stats, moved_at): (i32, &crate::movement::MoveStats, [f32; 3]),
    (target, target_mover): (i32, Option<&crate::movement::Mover>),
    range: f32,
) -> Result<bool, GenieError> {
    let me = spatial
        .get_known(id)?
        .ok_or(GenieError::unsupported("an entering unit without a place"))?;
    let them = spatial.get_known(target)?.ok_or(GenieError::unsupported(
        "an Enter's building without a place",
    ))?;
    let gap = footprint_gap_squared(me.position, me.radius, them.position, them.radius);
    let speed = target_mover.map_or(0.0, |m| m.stats.speed);
    let them_at = target_mover.map_or(them.position, |m| m.moved_at);
    EnterAction::in_reach(range, gap, speed, || {
        walled_off(
            [moved_at[0], moved_at[1]],
            [them_at[0], them_at[1]],
            spatial.map,
            |x, y| {
                spatial.point_clear(
                    id,
                    [x as f32, y as f32],
                    stats.radius[0],
                    stats.restriction_mask,
                )
            },
            |x, y| {
                let mut n = 0;
                for (_, o) in spatial.objects_on(x, y) {
                    let m = masters.get(o.player).and_then(|m| m.get(o.master)).ok_or(
                        GenieError::unsupported("an object on a tile of a master the slice lacks"),
                    )?;
                    n += usize::from(m.kind == BUILDING);
                }
                Ok(n)
            },
        )
    })
}

/// What an Enter's update reads of its target `t` (of master `tm`, with `room` for `unit`): the
/// [`EnterTarget`] (allied: its own player's; the reach by [`enter_reach`] with the task's
/// `range`), the footprints' gap (`FUN_14118f680`) and the unit's speed for a new `MoveTo`, and
/// for a transport where the two meet ([`meeting_point`]; nothing the update does before its
/// `set_state(4)` moves either, so it holds there).
#[allow(clippy::type_complexity)]
pub(crate) fn enter_view(
    spatial: &crate::spatial::Spatial,
    zones: &mut crate::zones::Zones,
    masters: &[Masters],
    unit: &crate::turn::Unit,
    (t, tm, room): (&crate::turn::Unit, &Master, bool),
    range: f32,
) -> Result<(EnterTarget, (f32, f32), Result<([f32; 2], u8), GenieError>), GenieError> {
    let building = tm.kind == BUILDING;
    let mover = unit.mover.as_ref();
    let stats = mover.map(|m| m.stats).ok_or(GenieError::unsupported(
        "an Enter's MoveTo of a unit without speed",
    ))?;
    let moved_at = mover.map_or(unit.position, |m| m.moved_at);
    let target_mover = if building { None } else { t.mover.as_ref() };
    let in_reach = enter_reach(
        spatial,
        masters,
        (unit.id, &stats, moved_at),
        (t.id, target_mover),
        range,
    )?;
    let (me, them) = (
        spatial.get(unit.id).expect("placed: in reach above"),
        spatial.get(t.id).expect("placed: in reach above"),
    );
    let gap = crate::movement::footprint_gap(me.position, me.radius, them.position, them.radius);
    let meeting = if building {
        Ok(([t.position[0], t.position[1]], 0))
    } else {
        meeting_target(t, tm, room)
            .and_then(|mt| meeting_point(&mt, (unit.id, unit.position, &stats), spatial, zones))
    };
    Ok((
        EnterTarget {
            state: t.life.state,
            allied: true,
            room,
            position: t.position,
            in_reach,
            building,
        },
        (gap, stats.speed),
        meeting,
    ))
}

/// A transport's slot 0x3d8 (`FUN_14114ffc0`) for an Enter: whether `ship` reaches within
/// [`ENTER_NEAR`] of the point's tile (the point truncated), [`crate::movement::Mover::reaches`]
/// from where it last moved to; one without movement state does not.
pub(crate) fn ship_reaches(
    ship: &crate::turn::Unit,
    world: &crate::spatial::Spatial,
    plan: &mut crate::movement::Planning<'_>,
    to: [f32; 3],
) -> Result<bool, GenieError> {
    let Some(m) = ship.mover.as_ref() else {
        return Ok(false);
    };
    let me = crate::movement::Me {
        id: ship.id,
        position: ship.position,
        action_state: ship.top_action().map_or(0, |a| a.state),
        group: ship.group.id,
    };
    let goal = [cvttss2si(to[0]) as f32, cvttss2si(to[1]) as f32];
    m.reaches(&me, world, plan, goal, ENTER_NEAR, ship.life.master_id)
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
pub(crate) mod tests {
    use super::*;
    use crate::masters::BuildingMaster;
    use crate::turn::{Player, Unit};

    pub(crate) const TC: i16 = 109;
    pub(crate) const VILLAGER: i16 = 83;
    pub(crate) const ARCHER: i16 = 4;
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

    pub(crate) fn unit(id: i32, master: i16, hp: f32) -> Unit {
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
        // MarvinBlue/426048731's 9171 garrisons the town centre at (104, 47) from 3.5 tiles off:
        // set_state(4)'s MoveTo to it (FUN_140aa0df0), quiet, starts in state 3
        let tc = EnterTarget {
            state: 2,
            allied: true,
            room: true,
            position: [104.0, 47.0, 0.0],
            in_reach: false,
            building: true,
        };
        let never = || -> Result<([f32; 2], u8), GenieError> { panic!("a building's point") };
        let mut far = EnterAction::new(9149, [104.0, 47.0, 0.0], 1.0, 3);
        assert_eq!(
            far.walk_again(&tc, 3.5, 0.8, &mut never.clone()),
            Ok(Walking::Walk(None))
        );
        assert_eq!((far.state, far.move_to), (WALK, Some(MoveTo::Start)));
        // within ARRIVED of its footprint the MoveTo starts arrived (state 1)
        let mut close = EnterAction::new(9149, [104.0, 47.0, 0.0], 1.0, 3);
        assert_eq!(
            close.walk_again(&tc, 0.0, 0.8, &mut never.clone()),
            Ok(Walking::Walk(None))
        );
        assert_eq!(close.move_to, Some(MoveTo::Arrived));
        // 10049 of update 4409 stands within the task's range: it enters at once
        let mut near = EnterAction::new(9149, [104.0, 47.0, 0.0], 1.0, 3);
        let in_reach = EnterTarget {
            in_reach: true,
            ..tc
        };
        assert_eq!(
            near.walk_again(&in_reach, 0.6, 0.8, &mut never.clone()),
            Ok(Walking::Enter)
        );
        assert_eq!(near.move_to, None);
        // the reach is the footprints' squared gap against the range squared (COMISS/JBE: a NaN
        // gap is in); a target with speed within a range of 1 (COMISS/JC: a NaN range is not
        // tested) asks the tiles about the two
        let no_tiles = || -> Result<bool, GenieError> { panic!("no tile test") };
        assert_eq!(EnterAction::in_reach(1.0, 1.0, 0.0, no_tiles), Ok(true));
        assert_eq!(EnterAction::in_reach(1.0, 1.01, 0.0, no_tiles), Ok(false));
        assert_eq!(
            EnterAction::in_reach(1.0, f32::NAN, 0.0, no_tiles),
            Ok(true)
        );
        assert_eq!(EnterAction::in_reach(1.5, 0.5, 0.8, no_tiles), Ok(true));
        assert_eq!(
            EnterAction::in_reach(f32::NAN, 0.5, 0.8, no_tiles),
            Ok(true)
        );
        assert_eq!(EnterAction::in_reach(1.0, 0.5, 0.8, || Ok(true)), Ok(false));
        assert_eq!(EnterAction::in_reach(1.0, 0.5, 0.8, || Ok(false)), Ok(true));
        assert_eq!(
            EnterAction::in_reach(1.0, 0.5, f32::NAN, no_tiles),
            Ok(true)
        );
    }

    #[test]
    fn a_transport_s_enter_walks_to_it_and_sends_it_where_they_meet() {
        // FUN_140a87320 case 4 on a transport: the point and code of FUN_140a85f30; code 1 sends
        // the transport there at its own height, code 0 does not; the MoveTo goes to the
        // transport within 0
        let ship = EnterTarget {
            state: 2,
            allied: true,
            room: true,
            position: [79.5, 92.5, 0.25],
            in_reach: false,
            building: false,
        };
        let mut enter = EnterAction::new(7420, [79.5, 92.5, 0.0], 1.0, 20);
        let walking = enter.walk_again(&ship, 6.0, 0.8, &mut || Ok(([82.5, 89.5], 1)));
        assert_eq!(walking, Ok(Walking::Walk(Some([82.5, 89.5, 0.25]))));
        assert_eq!(
            (enter.point, enter.move_to),
            ([82.5, 89.5, 0.0], Some(MoveTo::Start))
        );
        assert_eq!(enter.walk_range(false), 0.0);
        assert_eq!(enter.walk_range(true), 1.0);
        let walking = enter.walk_again(&ship, 6.0, 0.8, &mut || Ok(([80.0, 91.0], 0)));
        assert_eq!(walking, Ok(Walking::Walk(None)));
        assert_eq!(enter.point, [80.0, 91.0, 0.0]);
        // a unit without speed beyond ARRIVED: the MoveTo's state 13, not modelled
        assert!(
            enter
                .walk_again(&ship, 6.0, 0.0, &mut || Ok(([80.0, 91.0], 0)))
                .is_err()
        );
    }

    /// A transport target, room for the unit, 1.0 tiles off its point on y.
    fn moved_ship(y: f32) -> EnterTarget {
        EnterTarget {
            state: 2,
            allied: true,
            room: true,
            position: [82.5, y, 0.0],
            in_reach: false,
            building: false,
        }
    }

    /// A walk of unit 1 without movement state or world, as [`EnterAction::update`] takes it.
    fn idle_walk(world: &mut crate::spatial::Spatial) -> Walk<'_> {
        Walk {
            mover: None,
            me: crate::movement::Me {
                id: 1,
                position: [82.5, 87.5, 0.0],
                action_state: 4,
                group: -1,
            },
            dt: 0.5,
            world,
            plan: None,
        }
    }

    #[test]
    fn a_transport_that_moved_off_the_point_makes_the_enter_walk_again() {
        // FUN_140a87a80 state 4: FUN_140a7bbf0 (not on the point) and FUN_140a7bd10 (more than
        // half a tile off: COMISS/JA) set_state(4); then the new MoveTo's update; the transport's
        // order waits on slot 0x3d8
        let mut world = crate::spatial::Spatial::new([100, 100]);
        let point = [82.5, 89.5, 0.0];
        let mut enter = EnterAction {
            move_to: Some(MoveTo::Arrived),
            ..EnterAction::new(7420, point, 1.0, 20)
        };
        let reached = std::cell::Cell::new(None);
        let step = enter
            .update(
                idle_walk(&mut world),
                Some(moved_ship(90.5)),
                (0.0, 0.8),
                || Ok(([82.5, 89.5], 1)),
                |_, to| {
                    reached.set(Some(to));
                    Ok(true)
                },
                (false, 0),
            )
            .unwrap();
        assert_eq!(reached.get(), Some([82.5, 89.5, 0.0]));
        assert_eq!(step.order, Some([82.5, 89.5, 0.0]));
        // the new MoveTo starts arrived (gap 0): code 1 out of reach waits (set_state(2))
        assert_eq!((enter.state, enter.timer, step.waits), (WAIT, 1.0, true));
        // within half a tile on each axis it does not walk again; a NaN is not off
        for y in [90.0, f32::NAN] {
            let mut enter = EnterAction {
                move_to: Some(MoveTo::Arrived),
                ..EnterAction::new(7420, point, 1.0, 20)
            };
            let step = enter
                .update(
                    idle_walk(&mut world),
                    Some(moved_ship(y)),
                    (0.0, 0.8),
                    || panic!("no walk again"),
                    |_, _| panic!("no order"),
                    (false, 0),
                )
                .unwrap();
            assert_eq!(step.order, None);
        }
        // a transport that cannot reach the point: set_state(13), not modelled
        let mut enter = EnterAction {
            move_to: Some(MoveTo::Arrived),
            ..EnterAction::new(7420, point, 1.0, 20)
        };
        let failed = enter.update(
            idle_walk(&mut world),
            Some(moved_ship(90.5)),
            (0.0, 0.8),
            || Ok(([82.5, 89.5], 1)),
            |_, _| Ok(false),
            (false, 0),
        );
        assert_eq!(failed, Err(UNREACHED));
    }

    /// The Enter's waiting `MoveTo` (state 2, after the motion step gave up): the world's dt
    /// onto its timer (code 0: nothing else happens); at its delay it asks again.
    #[test]
    fn an_enter_s_waiting_move_to_asks_again_at_its_delay() {
        let mut world = crate::spatial::Spatial::new([100, 100]);
        let ship = moved_ship(89.5);
        let mut enter = EnterAction {
            move_to: Some(MoveTo::Waiting),
            move_wait: crate::move_to::MoveWait {
                state: 1,
                timer: 0.0,
                delay: 1.0,
            },
            ..EnterAction::new(7420, ship.position, 1.0, 20)
        };
        let dt = idle_walk(&mut world).dt;
        let step = enter
            .update(
                idle_walk(&mut world),
                Some(ship),
                (0.0, 0.8),
                || panic!("no walk again"),
                |_, _| panic!("no order"),
                (true, 0),
            )
            .unwrap();
        assert_eq!(step, EnterStep::default());
        assert_eq!(
            (enter.move_to, enter.move_wait.timer),
            (Some(MoveTo::Waiting), dt)
        );
        enter.move_wait.timer = 1.0;
        let _ = enter
            .update(
                idle_walk(&mut world),
                Some(ship),
                (0.0, 0.8),
                || panic!("no walk again"),
                |_, _| panic!("no order"),
                (true, 0),
            )
            .unwrap();
        assert_eq!(
            (enter.move_to, enter.move_wait.timer),
            (Some(MoveTo::Start), 0.0)
        );
    }

    #[test]
    fn a_walk_again_into_reach_enters_and_the_update_enters_again() {
        // the set_state(4) of a moved transport finds the unit in reach: FUN_140a867b0, then the
        // update's code 1 (no MoveTo) in reach enters again
        let mut world = crate::spatial::Spatial::new([100, 100]);
        let mut enter = EnterAction {
            move_to: Some(MoveTo::Arrived),
            ..EnterAction::new(7420, [82.5, 89.5, 0.0], 1.0, 20)
        };
        let ship = EnterTarget {
            in_reach: true,
            ..moved_ship(90.5)
        };
        let step = enter
            .update(
                idle_walk(&mut world),
                Some(ship),
                (0.0, 0.8),
                || panic!("in reach: no point"),
                |_, _| panic!("no order"),
                (false, 0),
            )
            .unwrap();
        assert_eq!((step.enter, step.again, enter.move_to), (true, true, None));
        // without the walk again, code 1 in reach enters once
        let mut enter = EnterAction {
            move_to: Some(MoveTo::Arrived),
            ..EnterAction::new(7420, [82.5, 90.5, 0.0], 1.0, 20)
        };
        let step = enter
            .update(
                idle_walk(&mut world),
                Some(ship),
                (0.0, 0.8),
                || panic!("no walk again"),
                |_, _| panic!("no order"),
                (false, 0),
            )
            .unwrap();
        assert_eq!((step.enter, step.again), (true, false));
    }

    #[test]
    fn a_failed_move_walks_again_once_then_waits_and_a_wait_walks_again() {
        // code 3: +0x68 lets one set_state(4); the next waits a second (set_state(2)), whose end
        // walks again
        let mut world = crate::spatial::Spatial::new([100, 100]);
        let ship = moved_ship(89.5);
        let mut enter = EnterAction {
            move_to: Some(MoveTo::Failed),
            ..EnterAction::new(7420, [82.5, 89.5, 0.0], 1.0, 20)
        };
        let run = |enter: &mut EnterAction, world: &mut crate::spatial::Spatial| {
            enter.update(
                idle_walk(world),
                Some(ship),
                (2.0, 0.8),
                || Ok(([82.5, 89.5], 0)),
                |_, _| panic!("code 0: no order"),
                (false, 0),
            )
        };
        let step = run(&mut enter, &mut world).unwrap();
        assert_eq!(
            (enter.retry, enter.move_to, step.waits),
            (false, Some(MoveTo::Start), false)
        );
        enter.move_to = Some(MoveTo::Failed);
        let step = run(&mut enter, &mut world).unwrap();
        assert_eq!((enter.state, enter.move_to, step.waits), (WAIT, None, true));
        // the wait counts down by dt (0.5): at 0 it walks again
        let step = run(&mut enter, &mut world).unwrap();
        assert_eq!((enter.state, enter.timer, step.waits), (WAIT, 0.5, false));
        let _ = run(&mut enter, &mut world).unwrap();
        assert_eq!((enter.state, enter.move_to), (WALK, Some(MoveTo::Start)));
    }

    #[test]
    fn a_meeting_point_is_the_shore_on_the_way_else_a_beach_under_the_unit() {
        // FUN_140a85f30 on an idle transport with room and speed
        let ship = MeetingTarget {
            room: true,
            speed: 1.1,
            state: 0,
            class: 20,
            position: [79.5, 92.5],
            action_point: Err(UNPLACED),
        };
        let unit = [83.5, 86.25];
        let no_beach = || panic!("no beach test");
        assert_eq!(
            meeting(&ship, unit, || Ok(Some([81, 89])), no_beach),
            Ok(([81.5, 89.5], 1))
        );
        // no shore on the way: a beach under the unit gives its place rounded (floorf of x+0.5)
        assert_eq!(
            meeting(&ship, unit, || Ok(None), || true),
            Ok(([84.0, 86.0], 1))
        );
        assert_eq!(
            meeting(&ship, unit, || Ok(None), || false),
            Ok(([79.5, 92.5], 0))
        );
        // no room, no speed (or a negative one; a NaN has some): its place
        let at_place = |t: MeetingTarget| meeting(&t, unit, || panic!("no landing"), || true);
        for t in [
            MeetingTarget {
                room: false,
                ..ship.clone()
            },
            MeetingTarget {
                speed: 0.0,
                ..ship.clone()
            },
            MeetingTarget {
                speed: -1.0,
                ..ship.clone()
            },
        ] {
            assert_eq!(at_place(t), Ok(([79.5, 92.5], 0)));
        }
        let nan = MeetingTarget {
            speed: f32::NAN,
            ..ship.clone()
        };
        assert_eq!(
            meeting(&nan, unit, || Ok(Some([81, 89])), no_beach),
            Ok(([81.5, 89.5], 1))
        );
        // moving (state 4): where its action goes, but a siege weapon at its place; another
        // state at its place
        let moving = MeetingTarget {
            state: 4,
            action_point: Ok([60.0, 70.0]),
            ..ship
        };
        assert_eq!(at_place(moving.clone()), Ok(([60.0, 70.0], 0)));
        assert_eq!(
            at_place(MeetingTarget {
                class: 13,
                ..moving.clone()
            }),
            Ok(([79.5, 92.5], 0))
        );
        assert_eq!(
            at_place(MeetingTarget { state: 7, ..moving }),
            Ok(([79.5, 92.5], 0))
        );
    }

    #[test]
    fn the_landing_walk_steps_twice_per_tile_of_the_longer_axis() {
        // FUN_141157840 from (79, 92) to (83, 86): the y axis is longer (6), 12 steps of
        // (1/3, -1/2) in floats (79 + 3 steps is 80.00001); the tiles in the order they are
        // first reached
        let mut seen = Vec::new();
        let found = landing_tile([79, 92], [83, 86], |tile, _| {
            seen.push(tile);
            Ok(false)
        });
        assert_eq!(found, Ok(None));
        assert_eq!(
            seen,
            [
                [79, 91],
                [80, 90],
                [80, 89],
                [81, 89],
                [81, 88],
                [82, 87],
                [82, 86],
                [83, 86],
            ]
        );
        // the first that lands is the meeting tile, with the float place reached there
        let found = landing_tile([79, 92], [83, 86], |tile, [x, y]| {
            if tile == [81, 89] {
                assert_eq!([x, y], [81.000_015, 89.0]);
            }
            Ok(tile == [81, 89])
        });
        assert_eq!(found, Ok(Some([81, 89])));
        // equal axes: x is the longer (`<=`); one tile: none
        let mut seen = Vec::new();
        let _ = landing_tile([0, 0], [-2, 2], |tile, _| {
            seen.push(tile);
            Ok(false)
        });
        assert_eq!(seen, [[0, 0], [-1, 1], [-2, 2]]);
        assert_eq!(
            landing_tile([5, 5], [5, 5], |_, _| panic!("no step")),
            Ok(None)
        );
    }

    #[test]
    fn the_reach_tile_test_counts_buildings_where_the_unit_cannot_stand() {
        // FUN_140a85c30 for range <= 1 and a target with speed: the box of the two tiles (one
        // tile and the next when it spans more than one), the two tiles left out
        let mut asked = Vec::new();
        let walled = walled_off(
            [10.5, 10.5],
            [11.5, 11.5],
            [100, 100],
            |x, y| {
                asked.push((x, y));
                Ok(false)
            },
            |x, y| Ok(usize::from((x, y) == (10, 11) || (x, y) == (11, 10))),
        );
        assert_eq!(walled, Ok(true));
        assert_eq!(asked, [(10, 11), (11, 10)]);
        // a tile the unit could stand on does not count
        let walled = walled_off(
            [10.5, 10.5],
            [11.5, 11.5],
            [100, 100],
            |x, y| Ok((x, y) == (11, 10)),
            |_, _| Ok(1),
        );
        assert_eq!(walled, Ok(false));
        // far apart: the first tile of the box and the next, within the map
        let mut asked = Vec::new();
        let _ = walled_off(
            [98.5, 2.5],
            [95.5, 9.5],
            [97, 100],
            |x, y| {
                asked.push((x, y));
                Ok(true)
            },
            |_, _| Ok(0),
        );
        assert_eq!(asked, [(95, 2), (95, 3), (96, 2), (96, 3)]);
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
        // a unit of another type with no class flags 0x10002 (FUN_1411205e0) goes on as for a
        // building: the transport is of type 70, so it takes none
        assert_eq!(takes(&t, TC, true), Ok(false));
        // a unit of a type below 70 whose class has them (13: 0x3) takes the transport's branch
        let mut masters = garrison_masters();
        masters = Masters::new(masters.iter().cloned().chain([Master {
            id: 1500,
            kind: 60,
            class: 13,
            tasks: vec![task(GARRISON, 20)],
            ..Master::default()
        }]));
        t.masters = (0..4).map(|_| masters.clone()).collect();
        assert_eq!(takes(&t, 1500, true), Ok(true));
        // full: no room for one more
        assert_eq!(takes(&t, 1500, false), Ok(false));
    }

    #[test]
    fn the_class_flags_are_the_images_table_by_class() {
        // FUN_1411205e0 with DAT_1439e37b0: villagers (4, 0x10163), infantry (6, 0x10103) and
        // siege (13, 0x3) garrison a transport; ships (20, 21, 22) do not
        for class in [0, 4, 6, 13, 18, 44, 58] {
            assert!(class_has_flags(class, TRANSPORTED), "class {class}");
        }
        for class in [2, 20, 21, 22, 52, 60, 63] {
            assert!(!class_has_flags(class, TRANSPORTED), "class {class}");
        }
        // other flags of the same words; a class outside the table has none (JS, JGE)
        assert!(class_has_flags(17, 0x10));
        assert!(class_has_flags(21, 0x100));
        assert!(!class_has_flags(-1, u32::MAX));
        assert!(!class_has_flags(64, u32::MAX));
        assert!(class_has_flags(61, u32::MAX));
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
    pub(crate) fn let_out_world() -> Turns {
        sized_world(20, 20)
    }

    /// [`let_out_world`] on a map of `w` × `h` tiles.
    pub(crate) fn sized_world(w: i32, h: i32) -> Turns {
        let mut t = garrison_world();
        let mut masters = garrison_masters();
        if let Some(tc) = masters.get_mut(TC) {
            tc.radius = [2.0, 2.0];
            tc.tall = true;
            tc.volley_fire = 1.0;
            // no stack unit, head or annexes: the tracker files it under its own id
            if let Some(b) = tc.building.as_mut() {
                b.stack_unit = -1;
                b.head_unit = -1;
                b.annexes = [-1; 4];
            }
        }
        if let Some(v) = masters.get_mut(VILLAGER) {
            v.radius = [0.2, 0.2];
            v.tall = true;
            v.moving = Some(standing());
        }
        t.masters = (0..4).map(|_| masters.clone()).collect();
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

    /// `FUN_1411921d0`, the enter itself: a member of a unit group (`+0x6d0`) leaves the group
    /// the world holds (removeUnit with the building and 0, `FUN_14102c160`) and stands (slot
    /// 0x560) between leaving the mesh and joining the building.
    /// A new unit whose building's gather point is the building itself (`FUN_140ace9c0`): with
    /// fewer than 10 inside, or fewer than its capacity, its slot 0x1a0 forced (1:
    /// `FUN_1411921d0` passes over the room test, slot 0x198) takes it in at once: no notify,
    /// no state change, no move; the building's sight takes its circle of 5 and the villager's
    /// town-bell record its work.
    #[test]
    fn a_gather_point_on_its_own_building_takes_the_new_unit_in() {
        let mut t = let_out_world();
        walker(&mut t, 1, 1000, VILLAGER, [12.5, 10.0]);
        let idx = t.players[1].objects.len() - 1;
        let state = t.players[1].objects[idx].life.state;
        let g = crate::turn::GatherPoint {
            point: [10.0, 10.0, 0.0],
            target: 100,
            target_master: TC,
        };
        t.gather(1, idx, g, 100, true, true).unwrap();
        let v = &t.players[1].objects[idx];
        assert_eq!(
            (
                v.life.inside,
                v.life.state,
                v.move_to.is_some(),
                v.enter.is_some()
            ),
            (Some(100), state, false, false)
        );
        assert_eq!(v.town_bell.work.target, 100);
        assert_eq!(t.object(100).unwrap().1.garrison, [1000]);
    }

    #[test]
    fn a_grouped_unit_that_enters_a_building_leaves_its_group() {
        let mut t = let_out_world();
        let mut v = unit(400, VILLAGER, 25.0);
        v.position = [12.5, 10.0, 0.0];
        let mut walker = crate::hunt::tests::walker();
        walker.stats = standing();
        v.mover = Some(walker);
        v.group.id = 7;
        v.town_bell.flag = true;
        t.players[1].objects.push(v);
        t.spatial.insert(
            400,
            crate::spatial::Placed {
                position: [12.5, 10.0, 0.0],
                radius: [0.2, 0.2],
                class: 4,
                player: 1,
                master: VILLAGER,
                obstruction_type: 0,
                buildable_on: 0,
                tall: true,
                speed: 0.0,
                unknown: None,
                obstruction: None,
            },
        );
        let member = |id| crate::group::Member {
            id,
            ..crate::group::Member::default()
        };
        t.groups.groups.push(crate::group::UnitGroup {
            id: 7,
            owner: 1,
            members: vec![member(401), member(400)],
            ..crate::group::UnitGroup::default()
        });
        let idx = t.players[1].objects.len() - 1;
        let garrison = Task {
            work_range: 1.0,
            ..task(GARRISON, 3)
        };
        t.start_enter(1, idx, 100, &garrison, [10.0, 10.0, 0.0], false)
            .unwrap();
        let v = &t.players[1].objects[idx];
        assert_eq!((v.life.inside, v.group.id), (Some(100), -1));
        assert!(v.mover.as_ref().unwrap().speed_stopped);
        let members: Vec<i32> = t.groups.groups[0].members.iter().map(|m| m.id).collect();
        assert_eq!(members, [401]);
    }

    /// Puts unit `id` of player `p` and master `m` inside the town centre.
    pub(crate) fn inside_tc(t: &mut Turns, p: usize, id: i32, m: i16) {
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
        assert!(t.back_to_work(Some((1, 0)), 2000).unwrap());
        assert!(t.players[1].objects[0].firepower.is_nan());
        assert_eq!(t.players[1].objects[0].garrison, [1000]);
        t.players[1].objects[0].firepower = 0.5;
        t.object_mut(1000).unwrap().1.town_bell.work.point = [4.5, 15.5];
        assert!(t.back_to_work(Some((1, 0)), 1000).unwrap());
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
    pub(crate) fn walker(t: &mut Turns, p: usize, id: i32, m: i16, at: [f32; 2]) {
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
            user_waypoints: Vec::new(),
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

    /// A building of player 1 of master `m` (radius 2) at `at`, `annex_of` its main building.
    pub(crate) fn building_at(t: &mut Turns, id: i32, m: i16, at: [f32; 2], annex_of: Option<i32>) {
        let mut b = unit(id, m, 2400.0);
        b.position = [at[0], at[1], 0.0];
        b.annex_of = annex_of;
        t.players[1].objects.push(b);
        t.spatial.insert(
            id,
            crate::spatial::Placed {
                position: [at[0], at[1], 0.0],
                radius: [2.0, 2.0],
                class: 52,
                player: 1,
                master: m,
                obstruction_type: 2,
                buildable_on: 0,
                tall: true,
                speed: 0.0,
                unknown: None,
                obstruction: Some(crate::spatial::Obstruction { kind: 2, class: 2 }),
            },
        );
    }

    /// Player 1's tracker files `ids` under master `m`'s key, done, in that order.
    pub(crate) fn tracked(t: &mut Turns, m: i16, ids: &[i32]) {
        let k = t.tracker_key(1, m);
        for &id in ids {
            t.players[1].tracker.add(id, k);
            t.players[1].tracker.complete(id, k);
        }
    }

    #[test]
    fn the_shelters_near_a_target_follow_its_owners_tracker_and_count_annexes_and_gates() {
        // FUN_14161e370 with +0x1e: the town centres' list (109) in the tracker's order; 102 is
        // beyond ten tiles, annex 104 is no building but pushes its count (one unit inside)
        let mut t = sized_world(40, 40);
        building_at(&mut t, 101, TC, [15.0, 10.0], None);
        building_at(&mut t, 102, TC, [30.0, 30.0], None);
        building_at(&mut t, 104, TC, [10.0, 15.0], Some(101));
        let mut inside = unit(500, VILLAGER, 25.0);
        inside.life.inside = Some(104);
        t.players[1].objects.push(inside);
        tracked(&mut t, TC, &[104, 102, 101, 100]);
        assert_eq!(
            t.shelters_near(1, 100, TC).unwrap(),
            Shelters {
                buildings: vec![101, 100],
                transports: Vec::new(),
                counts: vec![1, 0, 0],
            }
        );
        // the gates' list: no gate is a building, each pushes its count
        building_at(&mut t, 301, GATE, [10.0, 6.0], None);
        t.spatial.insert(300, *t.spatial.get(301).unwrap());
        tracked(&mut t, GATE, &[300, 301]);
        assert_eq!(
            t.shelters_near(1, 301, GATE).unwrap(),
            Shelters {
                buildings: Vec::new(),
                transports: Vec::new(),
                counts: vec![0, 0],
            }
        );
    }

    #[test]
    fn a_garrison_near_its_target_reads_the_count_list_at_its_pairs_index() {
        // the town centres take one unit; the list [104, 101, 100] gives the buildings
        // [101, 100] and the counts [1, 0, 0]: the villager's nearest pair (101, index 0) reads
        // the annex's count 1, which is full, so it goes to 100 (index 1, count 0)
        let world = |with_annex: bool| {
            let mut t = sized_world(40, 40);
            for m in &mut t.masters {
                if let Some(tc) = m.get_mut(TC) {
                    tc.garrison = 1;
                }
            }
            building_at(&mut t, 101, TC, [16.0, 10.0], None);
            building_at(&mut t, 104, TC, [10.0, 15.0], Some(101));
            let mut inside = unit(500, VILLAGER, 25.0);
            inside.life.inside = Some(104);
            t.players[1].objects.push(inside);
            let listed: &[i32] = if with_annex {
                &[104, 101, 100]
            } else {
                &[101, 100]
            };
            tracked(&mut t, TC, listed);
            walker(&mut t, 1, 1000, VILLAGER, [19.5, 10.5]);
            t.garrison_order(1, &[1000], 100, false, true).unwrap();
            t.object(1000).unwrap().1.enter.map(|e| e.target)
        };
        assert_eq!(world(true), Some(100));
        assert_eq!(world(false), Some(101));
    }

    #[test]
    fn a_garrison_through_the_unit_ais_queues_their_enter_orders() {
        // FUN_141621220 with param_7: slot 0x30 into slot 0x28 with 0x2cd, issued by the unit
        let mut t = let_out_world();
        walker(&mut t, 1, 1000, VILLAGER, [5.5, 5.5]);
        t.garrison_order(1, &[1000], 100, true, false).unwrap();
        let v = t.object(1000).unwrap().1;
        assert_eq!(v.enter, None);
        assert_eq!(
            v.ai.as_ref().unwrap().orders,
            [crate::unit_ai::QueuedOrder {
                issuer: 1000,
                order: ENTER_ORDER,
                priority: 100,
                target: 100,
                target_owner: 1,
                point: [10.0, 10.0, 0.0],
                range: 0.0,
                shown: [10.0, 10.0],
                ready: true,
                target_type: i32::from(TC),
            }]
        );
    }

    /// [`let_out_world`] with ship 200 at (`x`, `y`) on the map.
    fn ship_at(t: &mut Turns, at: [f32; 2]) {
        let ship = t.object_mut(200).expect("the ship").1;
        ship.position = [at[0], at[1], 0.0];
        t.spatial.insert(
            200,
            crate::spatial::Placed {
                position: [at[0], at[1], 0.0],
                radius: [0.5, 0.5],
                class: 20,
                player: 1,
                master: SHIP,
                obstruction_type: 0,
                buildable_on: 0,
                tall: true,
                speed: 1.0,
                unknown: None,
                obstruction: None,
            },
        );
    }

    #[test]
    fn a_garrison_into_a_transport_pairs_the_units_whose_class_has_the_flags() {
        // FUN_141621220: a villager (class 4, flags 0x10163) with a garrison task on the ship's
        // class 20 pairs with it and, through the unit AIs, queues its enter order on it; the
        // archer (class 0) has no such task, and the ship itself (class 20) no class flags
        let mut t = let_out_world();
        ship_at(&mut t, [5.5, 12.5]);
        walker(&mut t, 1, 1000, VILLAGER, [5.5, 5.5]);
        walker(&mut t, 1, 1001, ARCHER, [6.5, 5.5]);
        t.garrison_order(1, &[1000, 1001, 200], 200, true, false)
            .unwrap();
        let orders = |t: &Turns, id| {
            t.object(id).unwrap().1.ai.as_ref().map(|ai| {
                ai.orders
                    .iter()
                    .map(|o| (o.order, o.target))
                    .collect::<Vec<_>>()
            })
        };
        assert_eq!(orders(&t, 1000), Some(vec![(ENTER_ORDER, 200)]));
        assert_eq!(orders(&t, 1001), Some(vec![]));
        // a transport beyond the town bell's reach (param_6 1) pairs nothing
        let mut t = sized_world(40, 40);
        ship_at(&mut t, [5.5, 35.5]);
        walker(&mut t, 1, 1000, VILLAGER, [5.5, 5.5]);
        let shelters = Shelters {
            buildings: Vec::new(),
            transports: vec![200],
            counts: vec![0],
        };
        t.shelter(1, &[1000], &shelters, true, true).unwrap();
        assert_eq!(orders(&t, 1000), Some(vec![]));
        t.shelter(1, &[1000], &shelters, false, true).unwrap();
        assert_eq!(orders(&t, 1000), Some(vec![(ENTER_ORDER, 200)]));
        // with no unit the transport is never asked
        assert_eq!(t.garrison_order(1, &[], 200, false, false), Ok(()));
    }

    /// [`sized_world`] 40 × 40 with a beach row (terrain 1, flags 0x10) at y = 17 and water
    /// (terrain 2) from y = 18 on, every terrain passable for restriction 0; the villager's
    /// garrison task on transports (class 20) has range 1; ship 200 (speed 1.1, type 70) at
    /// `ship` and villager 1000 at `at`, each where it last moved to its place.
    fn shore_world(at: [f32; 2], ship: [f32; 2]) -> Turns {
        let mut t = sized_world(40, 40);
        for m in &mut t.masters {
            if let Some(v) = m.get_mut(VILLAGER) {
                v.tasks = vec![
                    task(GARRISON, 3),
                    Task {
                        work_range: 1.0,
                        ..task(GARRISON, 20)
                    },
                ];
            }
        }
        let tiles = (0..40 * 40)
            .map(|i| match i / 40 {
                17 => (1, 255, -1),
                y if 18 <= y => (2, 255, -1),
                _ => (0, 255, -1),
            })
            .collect();
        let kind = |flags| crate::terrain::TerrainKind {
            shown: true,
            flags,
            forest: false,
        };
        let mut terrain = crate::terrain::Terrain::new(
            40,
            40,
            tiles,
            vec![kind(0x20), kind(0x10), kind(1)],
            vec![vec![1.0, 1.0, 1.0]],
        );
        terrain.set_elevations(vec![0; 40 * 40]);
        t.spatial.terrain = Some(terrain);
        ship_at(&mut t, ship);
        let mut mover = crate::hunt::tests::walker();
        mover.stats = crate::movement::MoveStats {
            speed: 1.1,
            ..standing()
        };
        mover.moved_at = [ship[0], ship[1], 0.0];
        t.object_mut(200).unwrap().1.mover = Some(mover);
        walker(&mut t, 1, 1000, VILLAGER, at);
        t.object_mut(1000)
            .unwrap()
            .1
            .mover
            .as_mut()
            .unwrap()
            .moved_at = [at[0], at[1], 0.0];
        t
    }

    #[test]
    fn a_villager_sent_into_a_ship_meets_it_at_the_shore_on_the_way() {
        // FUN_14161e370 (no +0x1e) -> FUN_141621220's transports -> slot 0x7e8 with filter 3 ->
        // the finder's LAB_1412796b4 -> the Enter: FUN_140a85f30 walks from the ship's tile
        // (5, 30) to the villager's (5, 12) and meets it on the beach tile (5, 17), code 1: the
        // ship reaches within 1 of (5, 17) (slot 0x3d8, the long pather's answer) and moves
        // there (slot 0x340: a MoveTo, running, range 0)
        let mut t = shore_world([5.5, 12.5], [5.5, 30.5]);
        t.garrison_order(1, &[1000], 200, false, false).unwrap();
        let ship = t.object(200).unwrap().1.move_to.unwrap();
        assert_eq!(
            (ship.goal, ship.range, ship.run, ship.target),
            (Some([5.5, 17.5, 0.0]), 0.0, true, -1)
        );
        let enter = t.object(1000).unwrap().1.enter.unwrap();
        assert_eq!(
            (enter.target, enter.point, enter.state, enter.move_to),
            (200, [5.5, 17.5, 0.0], WALK, Some(MoveTo::Start))
        );
        // the unit AI holds the enter order and action, the ship its target
        let ai = t.object(1000).unwrap().1.ai.clone().unwrap();
        assert_eq!(
            (ai.current_order, ai.current_action, ai.current_target),
            (ENTER_ORDER, ENTER_ACTION, 200)
        );
    }

    #[test]
    fn a_villager_meets_a_moving_ship_where_it_goes() {
        // FUN_140a85f30 on a ship under a MoveTo (state 3): its goal, code 0, no order
        let mut t = shore_world([5.5, 12.5], [5.5, 30.5]);
        let ship = t.object_mut(200).unwrap().1;
        ship.move_to = Some(crate::move_to::MoveToAction {
            run: false,
            state: MoveTo::Start,
            goal: Some([9.5, 19.5, 0.0]),
            target: -1,
            range: 0.0,
            wait: crate::move_to::MoveWait::default(),
        });
        t.garrison_order(1, &[1000], 200, false, false).unwrap();
        let enter = t.object(1000).unwrap().1.enter.unwrap();
        assert_eq!(
            (enter.point, enter.move_to),
            ([9.5, 19.5, 0.0], Some(MoveTo::Start))
        );
        assert_eq!(
            t.object(200).unwrap().1.move_to.unwrap().goal,
            Some([9.5, 19.5, 0.0])
        );
        // the same order again: the work command finds the Enter's own task, whose slot 9 (the
        // base's, 0x140a7bf40) takes no new target, so a new Enter replaces it
        t.object_mut(1000).unwrap().1.enter.as_mut().unwrap().retry = false;
        t.garrison_order(1, &[1000], 200, false, false).unwrap();
        let enter = t.object(1000).unwrap().1.enter.unwrap();
        assert_eq!((enter.target, enter.retry), (200, true));
    }

    #[test]
    fn a_villager_beside_a_ship_enters_it_at_once() {
        // in reach (gap 0.8, range 1) of a ship with speed: the tiles (5, 19) and (5, 20) are
        // the two left out, no building between; FUN_140a867b0 -> FUN_1411921d0 -> the ship's
        // slot 0x1c0 (FUN_14120b790): inside at the ship's place; FUN_1412067f0 with 3 records
        // the Enter (the ship at its place, the enter action) and leaves the flag clear
        let mut t = shore_world([5.5, 19.0], [5.5, 20.5]);
        t.garrison_order(1, &[1000], 200, false, false).unwrap();
        let v = t.object(1000).unwrap().1;
        assert_eq!(v.life.inside, Some(200));
        assert_eq!([v.position[0], v.position[1]], [5.5, 20.5]);
        assert_eq!(v.enter.map(|e| e.state), Some(ENTERED));
        assert!(!v.town_bell.flag);
        assert_eq!(
            v.town_bell.work,
            crate::town_bell::BackToWork {
                target: 200,
                target_type: -1,
                action: ENTER_ACTION,
                target2: -1,
                point: [5.5, 20.5],
            }
        );
        assert_eq!(t.object(200).unwrap().1.garrison, [1000]);
        // the transport's count takes it: no room for another
        let (bp, b) = t.object(200).unwrap();
        assert!(!t.has_room(bp, b, None).unwrap());
    }

    #[test]
    fn a_villager_enters_an_allys_ship_with_its_owners_masters() {
        // player 2's ship beside player 1's villager: FUN_14120eb10 lets an ally's transport take
        // it (both ways allied), and the enter adds the villager to the ship's list
        let mut t = shore_world([5.5, 19.0], [5.5, 20.5]);
        let k = t.players[1]
            .objects
            .iter()
            .position(|u| u.id == 200)
            .unwrap();
        let ship = t.players[1].objects.remove(k);
        t.players[2].objects.push(ship);
        let mut placed = *t.spatial.get(200).unwrap();
        placed.player = 2;
        t.spatial.insert(200, placed);
        t.garrison_order(1, &[1000], 200, false, false).unwrap();
        assert_eq!(t.object(1000).unwrap().1.life.inside, Some(200));
        assert_eq!(t.object(200).unwrap().1.garrison, [1000]);
        // an enemy's ship takes no one in (player 3)
        let mut t = shore_world([5.5, 19.0], [5.5, 20.5]);
        let k = t.players[1]
            .objects
            .iter()
            .position(|u| u.id == 200)
            .unwrap();
        let ship = t.players[1].objects.remove(k);
        t.players[3].objects.push(ship);
        t.garrison_order(1, &[1000], 200, false, false).unwrap();
        assert_eq!(t.object(1000).unwrap().1.enter, None);
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
        t.settings.cheats_enabled = true;
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

    /// `FUN_1411e7220` toward no point from a building without a gather point (nor a command
    /// queue): the unit comes out, its unit AI stops (slot 0xb0 with 1: its stopAction), forgets
    /// its target and takes its stance again at its new place (slot 0x1c0 with `+0x1a9`).
    #[test]
    fn a_unit_let_out_toward_nothing_stops_and_keeps_its_stance() {
        let mut t = let_out_world();
        inside_tc(&mut t, 1, 1000, VILLAGER);
        let v = t.object_mut(1000).unwrap().1;
        v.ai = Some(crate::unit_ai::UnitAi {
            current_action: 0x261,
            current_target: 77,
            stance: 1,
            ..crate::unit_ai::UnitAi::default()
        });
        assert_eq!(
            t.let_out(1, 0, 1000, [NO_POINT, NO_POINT], (None, false)),
            Ok(true)
        );
        let v = t.object(1000).unwrap().1;
        assert_eq!(v.life.inside, None);
        let ai = v.ai.as_ref().unwrap();
        assert_eq!(
            (
                ai.current_action,
                ai.current_target,
                ai.stance,
                ai.last_action
            ),
            (-1, -1, 1, 0x261)
        );
        assert_eq!(ai.state_position, [v.position[0], v.position[1]]);
    }

    #[test]
    fn a_buildings_count_takes_relics_in_and_leaves_raiders_unmodelled() {
        // FUN_1411d0ce0 = FUN_141213500 (the +0x2f8 count and +0x9a8) less +0xc2c, which counts
        // the units of type 70 that came in with +0x9a0 set (FUN_1411cb850)
        let mut t = garrison_world();
        let mut masters = garrison_masters();
        masters = Masters::new(masters.iter().cloned().chain([Master {
            id: 285,
            kind: 20,
            class: 1,
            ..Master::default()
        }]));
        t.masters = (0..4).map(|_| masters.clone()).collect();
        for (p, id, m) in [(0, 900, 285), (1, 1000, VILLAGER), (2, 2000, VILLAGER)] {
            let mut u = unit(id, m, 25.0);
            u.life.inside = Some(100);
            t.players[p].objects.push(u);
        }
        assert_eq!(t.garrisoned(100), Ok(3));
        // a raider inside a building: its +0xc2c share is not kept
        t.object_mut(1000).unwrap().1.raider_countdown = 20;
        assert!(t.garrisoned(100).is_err());
        // a transport has no +0xc2c term
        let mut u = unit(3000, VILLAGER, 25.0);
        u.life.inside = Some(200);
        u.raider_countdown = 20;
        t.players[1].objects.push(u);
        assert_eq!(t.garrisoned(200), Ok(1));
    }
}
