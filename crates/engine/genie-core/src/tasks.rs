//! The task finder (`TRIBE_Task_List` slot 2, `FUN_1412784d0`, with the `RGE_Task_List` finder
//! `FUN_1411a8ba0` and its test of a task on a target, slot 4 `FUN_1411a8620`): which task of a
//! master's task list a unit does on a target object. The work command (object slot 37,
//! `FUN_14106ff70`) asks it of the unit's own master and, through the task swap group, of the
//! player's other masters ([`crate::masters::Masters::work_master`]).
//!
//! The finder tries the special tasks first, each for the targets it is about (dropping off what
//! the unit carries, building its player's foundation, garrisoning, repairing, converting, healing
//! and the like), then the general search: a task for the target's combat level, else for its
//! master, else for its class, each the first in list order the task's own test passes, else a
//! task for the terrain under the target's point. The masters' classes and ids it compares against
//! are engine constants; the game stores them encoded (a class as its value − 0xeac, a master id as
//! its id + 0x609c), the port compares the decoded values and names the encoded ones.

use crate::GenieError;
use crate::masters::{Master, Task};

/// A building's master type (`'P'`).
const BUILDING: u8 = 80;

/// Classes (encoded `-0xeac + c`).
mod class {
    pub const ARTIFACT: i16 = 1; // -0xeab
    pub const TRADE_BOAT: i16 = 2; // -0xeaa
    pub const BUILDING: i16 = 3; // -0xea9
    pub const VILLAGER: i16 = 4; // -0xea8
    pub const PREY: i16 = 9; // -0xea3
    pub const SIEGE: i16 = 13; // -0xe9f
    pub const MONK: i16 = 18; // -0xe9a
    pub const TRANSPORT: i16 = 20; // -0xe98
    pub const FISHING: i16 = 21; // -0xe97
    pub const WARSHIP: i16 = 22; // -0xe96
    pub const CLASS_42: i16 = 42; // -0xe82
    pub const CLASS_43: i16 = 43; // -0xe81
    pub const CLASS_51: i16 = 51; // -0xe79
    pub const CLASS_53: i16 = 53; // -0xe77
    pub const CLASS_54: i16 = 54; // -0xe76
    pub const CLASS_55: i16 = 55; // -0xe75
    pub const LIVESTOCK: i16 = 58; // -0xe72
}

/// Master ids (encoded `+0x609c`).
mod id {
    pub const ID_45: i16 = 45; // 0x60c9
    pub const ID_84: i16 = 84; // 0x60f0
    pub const ID_109: i16 = 109; // 0x6109
    pub const ID_199: i16 = 199; // 0x6163
    pub const ID_448: i16 = 448; // 0x625c
    pub const ID_546: i16 = 546; // 0x62be
    pub const ID_1105: i16 = 1105; // 0x64ed
    pub const ID_1808: i16 = 1808; // 0x67ac
}

/// The task types the general search leaves out (`FUN_14109f8c0`: 151, 152, 154, 155, 157, 160).
const fn special_type(t: i16) -> bool {
    let d = t.wrapping_sub(0x97) as u16;
    d < 10 && (0x25bu32 >> d) & 1 != 0
}

/// What the finder reads of the unit that would do the task.
#[derive(Clone, Copy, Debug)]
pub struct Actor<'a> {
    /// The unit.
    pub id: i32,
    /// Its own master (the finder's `param_3`, also when another master's list is searched).
    pub master: &'a Master,
    /// The owner's player id (`+0x718`).
    pub player: usize,
    /// What it carries (`+0x308`, `+0x310`).
    pub held: (i16, f32),
    /// The owner's attribute 95 (`+0x2f8`), which some monk tasks read.
    pub attribute_95: f32,
    /// The owner's civ (`FUN_140e879e0` of `player+0x738`), which drop sites read.
    pub civ: u8,
}

/// What the finder reads of a target object.
#[derive(Clone, Copy, Debug)]
pub struct TaskTarget<'a> {
    /// The target.
    pub id: i32,
    /// Its master.
    pub master: &'a Master,
    /// Its player.
    pub player: usize,
    /// The object state (`+0x640`).
    pub state: u8,
    /// What it holds (`+0x308`, `+0x310`): a resource's kind and amount.
    pub held: (i16, f32),
    /// Its hit points and their most (slots 0xc0 and 200).
    pub hp: f32,
    /// Its most hit points (slot 200).
    pub max_hp: f32,
    /// `+0x68a`, `unit_attackable_flag` (`FUN_141188b10`).
    pub attackable: bool,
    /// `+0x790`, `parent_pasture_id`: the pasture it is in, -1 for none.
    pub pasture: i32,
    /// The type of the first action in its action list (`+0x880`), if any.
    pub top_action: Option<i16>,
}

/// What the finder reads of the world: the players' relations (`FUN_141663dc0`: whether `from`
/// counts `to` as its own or allied, the game's `with_ally` flag set or not) and the drop sites.
/// `FUN_141663dc0` for a player of diplomacy `relations` (`player+0x260`): one's own player
/// counts; Gaia never does; else relation 0 does, and 4 with the ally flag; relation 1 counts only
/// under allied victory (`FUN_1415e8e10`), which the slice does not model.
#[must_use]
pub fn allied(relations: &[u8], from: usize, to: usize, with_ally: bool) -> bool {
    if from == to {
        return true;
    }
    if to == 0 {
        return false;
    }
    match relations.get(to) {
        Some(0) => true,
        Some(4) => with_ally,
        _ => false,
    }
}

/// The players' relations and the drop sites, as the task finder reads them.
pub trait Diplomacy {
    /// Player `from` counts `to` as its own or allied (with its allies when `with_ally`).
    fn allied(&self, from: usize, to: usize, with_ally: bool) -> bool;
    /// `FUN_140fdd190`: a building of master `building` takes what `actor` carries.
    fn drops_at(&self, building: i16, actor: &Actor<'_>) -> bool;
}

/// One entry of the drop-site table (`DAT_143cc2948`, the game's `dropsites.json`): the building,
/// the workers it serves (their class, −1 any, or else a list of their masters), the civs it is
/// for (empty: all) and the resource kinds it takes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DropSite {
    /// The building's master.
    pub building: i16,
    /// The class of the units that drop there (−1 any).
    pub worker_class: i16,
    /// The unit masters that drop there (empty: all).
    pub workers: Vec<i16>,
    /// The civs it is for (empty: all).
    pub civs: Vec<u8>,
    /// `+8`, `auto_work` (1 when the key is missing): a builder whose building completes looks
    /// for more work around it (`FUN_1415cc4e0`).
    pub auto_work: bool,
    /// `+0x50`, `target_list`: the objects whose resources it takes, in the file's order.
    pub targets: Vec<DropTarget>,
    /// The entry has `accepts_livestock` (`FUN_140fdf480` keeps the key's presence): its building
    /// is one of the drop sites a hunted livestock is led to (`DAT_143f157f0`, in table order).
    pub livestock: bool,
}

/// A drop site's target (`FUN_140fdf480`'s record of 0x14 bytes; each −1 when its key is missing).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DropTarget {
    /// `object_group`: the class of the objects.
    pub class: i16,
    /// `worker_num`: the most workers such an object may have.
    pub workers: i32,
    /// `object_state`: the state such an object must be in.
    pub state: i32,
    /// `attribute_type`: the resource it gives.
    pub attribute: i16,
}

impl DropSite {
    /// `FUN_140fdd7b0`: the resource the drop site takes from an object of class `class` in
    /// state `state` with `workers` workers: its first target of that class decides, unless it
    /// caps the workers below the object's or wants another state (None).
    #[must_use]
    pub fn resource_of(&self, class: i16, state: u8, workers: i32) -> Option<i16> {
        let t = self.targets.iter().find(|t| t.class == class)?;
        if (0 <= t.workers && t.workers < workers) || (0 <= t.state && t.state != i32::from(state))
        {
            return None;
        }
        (0 <= t.attribute).then_some(t.attribute)
    }
}

/// `FUN_140fdd8a0`: the table's first entry for building `building`, the worker class `class`
/// and civ `civ` (an entry with civs is for those alone).
#[must_use]
pub fn drop_site(table: &[DropSite], building: i16, class: i16, civ: u8) -> Option<&DropSite> {
    table.iter().find(|e| {
        (e.civs.is_empty() || e.civs.contains(&civ))
            && e.building == building
            && e.worker_class == class
    })
}

/// `FUN_140fdd190` over the table: the first entry for the actor's civ and kind of worker whose
/// building is `building` decides, by whether it takes the kind the actor carries.
#[must_use]
pub fn drops_at(table: &[DropSite], building: i16, actor: &Actor<'_>) -> bool {
    for e in table {
        if !e.civs.is_empty() && !e.civs.contains(&actor.civ) {
            continue;
        }
        let worker = if e.workers.is_empty() {
            e.worker_class < 0 || e.worker_class == actor.master.class
        } else {
            e.workers.contains(&actor.master.id)
        };
        if worker && e.building == building {
            return e.targets.iter().any(|t| t.attribute == actor.held.0);
        }
    }
    false
}

/// A task's requirement (`FUN_1410622c0`): no enabling resource (−1), or the player's attribute of
/// that index above 0.
#[must_use]
pub fn enabled(attributes: &[f32], resource: i16) -> bool {
    resource == -1
        || usize::try_from(resource)
            .ok()
            .and_then(|r| attributes.get(r))
            .is_some_and(|&v| 0.0 < v)
}

/// One search over the list: the first task the predicate takes, in list order.
fn first(list: &Master, pred: impl Fn(&Task) -> bool) -> Option<&Task> {
    list.tasks.iter().find(|t| pred(t))
}

/// The filter: any task (−1), or one of this type.
const fn wanted(filter: i16, t: &Task) -> bool {
    filter == -1 || filter == t.action_type
}

/// The task's own test on a target (`FUN_1411a8620`): a task for a holder (`holding_attr` 1)
/// wants the target to hold a resource of its type (or any, for a task without one); garrisoning
/// (3) and type 106 never apply; attacks (7, 9) want an attackable target; the task's owner type
/// says whose target (1 own, 2 an enemy player's, 3 Gaia's, 4 own, mutually allied or Gaia's, 5
/// an enemy's or, for a unit whose master has a combat level, Gaia's, 6 another's); never the unit
/// itself, but for type 4.
fn applies(task: &Task, actor: &Actor<'_>, target: &TaskTarget<'_>, dip: &impl Diplomacy) -> bool {
    if task.holding_attr == 1 {
        let (kind, amount) = target.held;
        if kind == -1 || amount <= 0.0 {
            return false;
        }
        if task.attribute_type != kind
            && task.attribute_type3 != kind
            && (task.attribute_type != -1 || task.attribute_type3 != -1)
        {
            return false;
        }
    }
    match task.action_type {
        3 | 106 => return false,
        7 | 9 if !target.attackable => return false,
        _ => {}
    }
    let (me, them) = (actor.player, target.player);
    let ok = match task.owner_type {
        1 => them == me,
        2 => them != me && them != 0 && !dip.allied(me, them, true),
        3 => them == 0,
        // type 4 answers at once, the unit itself included
        4 => {
            return them == me
                || them == 0
                || (dip.allied(me, them, true) && dip.allied(them, me, true));
        }
        // of Gaia's targets only for a unit whose own master has a combat level
        5 => {
            if them == me || (them != 0 && dip.allied(me, them, true)) {
                false
            } else {
                them != 0 || actor.master.combat_level != 0
            }
        }
        6 => them != me,
        _ => true,
    };
    ok && actor.id != target.id
}

/// `FUN_1411a8ba0`, the general search: with a target, a task for its combat level (a target in
/// state below 3 whose master has one; tasks that have one and the special types left out), else
/// for its master, else for its class, each the first whose test passes and that the filter
/// takes; else, at a point on the map, a task for the terrain there.
fn general<'m>(
    list: &'m Master,
    actor: &Actor<'_>,
    target: Option<&TaskTarget<'_>>,
    terrain: Option<i16>,
    filter: i16,
    dip: &impl Diplomacy,
) -> Option<&'m Task> {
    if list.tasks.is_empty() {
        return None;
    }
    if let Some(t) = target {
        let usable = |task: &Task| {
            !special_type(task.action_type) && applies(task, actor, t, dip) && wanted(filter, task)
        };
        if t.state < 3
            && t.master.combat_level != 0
            && let Some(task) = first(list, |task| task.combat_level != 0 && usable(task))
        {
            return Some(task);
        }
        if let Some(task) = first(list, |task| task.object_id == t.master.id && usable(task)) {
            return Some(task);
        }
        if let Some(task) = first(list, |task| {
            task.object_group == t.master.class && usable(task)
        }) {
            return Some(task);
        }
    }
    let kind = terrain?;
    first(list, |task| {
        task.terrain as u16 == (kind as u16 & 0xff) && wanted(filter, task)
    })
}

/// The task finder with filter 3 (`FUN_1412784d0`'s first part, which mode 1 runs too): on its
/// player's or a mutually allied player's building in state 2 that garrisons, for a unit master
/// of type 70 or master 1808, the first garrison task on buildings
/// ([`crate::garrison::is_garrison_task`]) when the building's `garrison_type` takes the master
/// ([`crate::garrison::garrison_type_takes`], class 42 left out); none on a building whose top
/// action is `0x7c` or `0x7d` (`FUN_1411e8630`; a computer player's production comes first,
/// `FUN_1411d5520`), which is not modelled. Else none: the caller goes on to the rest of the
/// finder for a target whose master has object flag 0 (a transport, [`find_task`]).
fn garrison_task<'m>(
    list: &'m Master,
    actor: &Actor<'_>,
    target: Option<&TaskTarget<'_>>,
    dip: &impl Diplomacy,
) -> Result<Option<&'m Task>, GenieError> {
    let Some(t) = target else {
        return Ok(None);
    };
    let mutual =
        dip.allied(actor.player, t.player, true) && dip.allied(t.player, actor.player, true);
    if (t.player == actor.player || mutual)
        && t.player != 0
        && t.state == 2
        && t.master.kind == BUILDING
        && (list.kind == 70 || list.id == id::ID_1808)
        && t.master.garrison != 0
    {
        let garrison_type = t.master.building.map_or(0, |b| b.garrison_type);
        let takes = crate::garrison::garrison_type_takes(garrison_type, list.class, list.id, false);
        if let Some(task) = first(list, crate::garrison::is_garrison_task).filter(|_| takes) {
            if matches!(t.top_action, Some(0x7c | 0x7d)) {
                return Err(GenieError::unsupported(
                    "a garrison task on a building at work 0x7c or 0x7d (FUN_1411e8630)",
                ));
            }
            return Ok(Some(task));
        }
    }
    Ok(None)
}

/// The task of `list` (a master's task list) for `actor` on `target` (`FUN_1412784d0` with the
/// work command's arguments: the target's point, the filter (−1 any, else an action type) and mode
/// 0). `target` is an annex's main building already (the finder's first step, `+0xbb8`); `general`
/// the target the general search sees, which repeats that step (an annex of an annex: its main
/// building's main building). `terrain`: the terrain under the point when the map's terrain table
/// counts it, which only a task for a terrain reads.
pub fn find_task<'m>(
    list: &'m Master,
    actor: &Actor<'_>,
    target: Option<&TaskTarget<'_>>,
    general_target: Option<&TaskTarget<'_>>,
    terrain: Option<i16>,
    filter: i16,
    dip: &impl Diplomacy,
) -> Result<Option<&'m Task>, GenieError> {
    let own = actor.master;
    // the garrison filter (3): a building's garrison task; with none, a target whose master has
    // object flag 0 (a transport) goes on through the finder to its garrison into the target
    // (`LAB_1412796b4`), any other has no task
    if filter == 3 {
        if let Some(task) = garrison_task(list, actor, target, dip)? {
            return Ok(Some(task));
        }
        if target.is_none_or(|t| t.master.object_flags & 1 == 0) {
            return Ok(None);
        }
    }
    if let Some(t) = target
        && own.object_flags & 0x10 != 0
        && t.master.class == class::PREY
    {
        return Err(GenieError::unsupported(
            "the task finder on prey for a hunter master (+0x610)",
        ));
    }
    // LAB_141278e9f: the monk tasks for carried relics and conversions of villagers
    let (held_kind, held_amount) = actor.held;
    if held_kind == 0x39
        && target
            .is_some_and(|t| t.state == 2 && t.player == actor.player && t.master.id == id::ID_109)
        && let Some(task) = first(list, |task| task.action_type == 0x87)
    {
        return Ok(Some(task));
    }
    let monk_master = own.id == id::ID_448 || own.id == id::ID_546;
    if actor.attribute_95 == 2.0
        && monk_master
        && target.is_some_and(|t| t.master.class == class::VILLAGER)
    {
        return Err(GenieError::unsupported(
            "the task finder's monk conversion of villagers (attribute 95)",
        ));
    }
    if actor.attribute_95 == 3.0
        && monk_master
        && target.is_some_and(|t| t.master.class == class::BUILDING)
    {
        return Err(GenieError::unsupported(
            "the task finder's monk conversion of buildings (attribute 95)",
        ));
    }
    // a garrison into the target without a task for it ends the finder before its tail, after
    // the repair and task 0x6f (`LAB_1412796b4`'s flag, with any filter)
    let mut no_task = false;
    if let Some(t) = target {
        // dropping off what the unit carries at its player's finished drop site: the tasks of
        // gathering (5, 110, 111) for what it carries
        if held_kind != -1
            && 0.0 < held_amount
            && t.state == 2
            && t.player == actor.player
            && dip.drops_at(t.master.id, actor)
        {
            let farm_at_work = t.master.kind == BUILDING && t.top_action == Some(0x7d);
            if let Some(task) = first(list, |task| {
                let gathers = matches!(task.action_type, 5 | 0x6e);
                !(farm_at_work && gathers)
                    && task.attribute_type == held_kind
                    && matches!(task.action_type, 5 | 0x6e | 0x6f)
                    && wanted(filter, task)
            }) {
                return Ok(Some(task));
            }
            if filter == -1
                && (t.master.kind != BUILDING || t.top_action.is_some_and(|a| a != 0x7d))
            {
                return Ok(None);
            }
        }
        // LAB_1412795f5: building its player's foundation
        if t.player == actor.player && t.master.kind == BUILDING && t.state == 0 {
            return Ok(build_task(list, t, filter));
        }
        if t.master.id == id::ID_199 {
            return Err(GenieError::unsupported("the task finder on master 199"));
        }
        // LAB_141279cfa -> LAB_1412796b4: garrisoning, repairing, task 0x6f
        let allied = dip.allied(actor.player, t.player, false);
        let own_or_allied = t.player == actor.player
            || (dip.allied(actor.player, t.player, true)
                && dip.allied(t.player, actor.player, true)
                && allied);
        // the garrison into the target (a transport): the first task of action 3 on its class
        // (the whole shorts) the filter takes (any, or 3); with none, the garrison filter has
        // no task, and any filter ends the finder after the repair and task 0x6f below
        if own_or_allied
            && t.state < 3
            && t.master.object_flags & 1 != 0
            && (t.master.class == class::TRANSPORT || own.class != class::VILLAGER || filter == 3)
        {
            if let Some(task) = first(list, |task| {
                task.action_type == crate::order::GARRISON
                    && task.object_group == t.master.class
                    && wanted(filter, task)
            }) {
                return Ok(Some(task));
            }
            if filter == 3 {
                return Ok(None);
            }
            no_task = filter == -1;
        }
        let repairable =
            t.player == actor.player || t.player == 0 || dip.allied(actor.player, t.player, true);
        let repair_class = t.master.kind == BUILDING
            || [
                class::TRANSPORT,
                class::FISHING,
                class::WARSHIP,
                class::TRADE_BOAT,
                class::CLASS_53,
                class::SIEGE,
                class::CLASS_51,
                class::CLASS_54,
                class::CLASS_55,
            ]
            .contains(&t.master.class);
        if repairable && repair_class && t.hp < t.max_hp && t.state < 3 {
            return Err(GenieError::unsupported("the task finder's repair (106)"));
        }
        if t.state < 3 && (t.master.id == id::ID_45 || t.master.id == id::ID_84) {
            return Err(GenieError::unsupported("the task finder's task 0x6f"));
        }
    }
    if no_task {
        return Ok(None);
    }
    // the tail: conversions, task 0x84, healing, task 0x88, the siege and class-53 tasks
    let monk = own.class == class::MONK;
    let converts = list.tasks.iter().any(|t| t.action_type == 0x68);
    if target.is_some() && (monk || (converts && filter == 0x68)) {
        return Err(GenieError::unsupported(
            "the task finder's conversion (0x68)",
        ));
    }
    if list.tasks.iter().any(|t| t.action_type == 0x84)
        && target.is_some_and(|t| t.master.class == class::CLASS_42)
    {
        return Err(GenieError::unsupported("the task finder's task 0x84"));
    }
    if list.tasks.iter().any(|t| t.action_type == 0x69) && target.is_some() {
        return Err(GenieError::unsupported("the task finder's healing (0x69)"));
    }
    if own.class == class::CLASS_43 && target.is_some() {
        return Err(GenieError::unsupported("the task finder's task 0x88"));
    }
    if own.class == class::CLASS_53 && target.is_some() {
        return Err(GenieError::unsupported(
            "the task finder's class-53 conversion",
        ));
    }
    let mut inside = false;
    if let Some(t) = target {
        if own.class == class::SIEGE && own.id == id::ID_1105 {
            return Err(GenieError::unsupported("the task finder for master 1105"));
        }
        // LAB_14127ba9e: a list with task 13 takes it, under that filter alone, for an own,
        // allied or artifact target in a state below 3
        if t.state < 3
            && (t.player == actor.player
                || dip.allied(actor.player, t.player, true)
                || t.master.class == class::ARTIFACT)
            && let Some(task) = first(list, |task| task.action_type == 0xd)
        {
            return Ok((filter == 0xd).then_some(task));
        }
        inside = t.pasture != -1;
    }
    // LAB_14127bb9f: a villager on livestock (or on what is inside something) takes task 110
    // for class 58; anything else the general search
    if let Some(t) = target
        && (t.master.class == class::LIVESTOCK || inside)
        && own.class == class::VILLAGER
    {
        return Ok(first(list, |task| {
            task.action_type == 0x6e
                && (task.object_group == class::LIVESTOCK || inside)
                && wanted(filter, task)
        }));
    }
    Ok(general(list, actor, general_target, terrain, -1, dip))
}

/// Building a foundation (`0x1412795f5`; the building's action check `FUN_1411e8630` holds for a
/// foundation without actions): of the Build tasks (101) the filter takes, the one for the
/// foundation's master, else the last for its class, each only with a work value, else the last
/// for any target.
fn build_task<'m>(list: &'m Master, t: &TaskTarget<'_>, filter: i16) -> Option<&'m Task> {
    let (mut by_class, mut generic) = (None, None);
    for task in list
        .tasks
        .iter()
        .filter(|t| t.action_type == 0x65 && wanted(filter, t))
    {
        if task.object_id == t.master.id {
            return (0.0 < task.work_val1).then_some(task);
        }
        if task.object_group == t.master.class {
            by_class = Some(task);
        }
        if task.object_group == -1 && task.object_id == -1 {
            generic = Some(task);
        }
    }
    by_class.map_or(generic, |task| (0.0 < task.work_val1).then_some(task))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every player on its own; the drop sites take wood (0) at master 562.
    struct Alone;

    impl Diplomacy for Alone {
        fn allied(&self, from: usize, to: usize, _: bool) -> bool {
            from == to
        }

        fn drops_at(&self, building: i16, actor: &Actor<'_>) -> bool {
            building == 562 && actor.held.0 == 0
        }
    }

    fn task(action_type: i16, object_group: i16, object_id: i16, work_val1: f32) -> Task {
        Task {
            action_type,
            object_group,
            object_id,
            work_val1,
            ..Task::default()
        }
    }

    fn master(id: i16, kind: u8, class: i16, tasks: Vec<Task>) -> Master {
        Master {
            id,
            kind,
            class,
            tasks,
            ..Master::default()
        }
    }

    fn actor(own: &Master, held: (i16, f32)) -> Actor<'_> {
        Actor {
            id: 9181,
            master: own,
            player: 2,
            held,
            attribute_95: 0.0,
            civ: 1,
        }
    }

    fn building(site: &Master, player: usize, state: u8) -> TaskTarget<'_> {
        TaskTarget {
            id: 10028,
            master: site,
            player,
            state,
            held: (-1, 0.0),
            hp: 550.0,
            max_hp: 550.0,
            attackable: true,
            pasture: -1,
            top_action: None,
        }
    }

    fn find<'m>(
        list: &'m Master,
        held: (i16, f32),
        t: &TaskTarget<'_>,
    ) -> Result<Option<&'m Task>, GenieError> {
        find_task(list, &actor(list, held), Some(t), Some(t), None, -1, &Alone)
    }

    #[test]
    fn a_build_task_for_the_foundation_s_master_or_class_needs_a_work_value() {
        let house = master(70, BUILDING, class::BUILDING, vec![]);
        let site = building(&house, 2, 0);
        let by = |tasks: Vec<Task>| {
            let builder = master(118, 70, class::VILLAGER, tasks);
            find(&builder, (-1, 0.0), &site).map(Option::<&Task>::copied)
        };
        let generic = task(101, -1, -1, 0.0);
        assert_eq!(by(vec![generic]), Ok(Some(generic)));
        assert_eq!(
            by(vec![task(101, 3, -1, 2.0), generic]),
            Ok(Some(task(101, 3, -1, 2.0)))
        );
        assert_eq!(by(vec![task(101, 3, -1, 0.0), generic]), Ok(None));
        assert_eq!(by(vec![generic, task(101, -1, 70, 0.0)]), Ok(None));
    }

    #[test]
    fn a_unit_carrying_what_a_finished_drop_site_takes_drops_it_off_there() {
        let camp = master(562, BUILDING, class::BUILDING, vec![]);
        let chop = Task {
            attribute_type: 0,
            ..task(5, 15, -1, 0.6)
        };
        let lumberjack = master(123, 70, class::VILLAGER, vec![task(101, -1, -1, 1.0), chop]);
        assert_eq!(
            find(&lumberjack, (0, 10.0), &building(&camp, 2, 2)),
            Ok(Some(&chop))
        );
        // nothing carried: the general search, which has no task for the camp
        assert_eq!(
            find(&lumberjack, (-1, 0.0), &building(&camp, 2, 2)),
            Ok(None)
        );
        // the camp still a foundation: building it
        assert_eq!(
            find(&lumberjack, (0, 10.0), &building(&camp, 2, 0)),
            Ok(Some(&task(101, -1, -1, 1.0)))
        );
    }

    /// A transport of player `player` (object flag 0) in state 2 of class `class`.
    fn transport(ship: &Master, player: usize) -> TaskTarget<'_> {
        TaskTarget {
            id: 7420,
            hp: 100.0,
            max_hp: 100.0,
            ..building(ship, player, 2)
        }
    }

    fn find_with<'m>(
        list: &'m Master,
        t: &TaskTarget<'_>,
        filter: i16,
    ) -> Result<Option<&'m Task>, GenieError> {
        find_task(
            list,
            &actor(list, (-1, 0.0)),
            Some(t),
            Some(t),
            None,
            filter,
            &Alone,
        )
    }

    #[test]
    fn the_garrison_filter_goes_on_to_a_transport_s_class_task() {
        // FUN_1412784d0 with filter 3 on a transport ship (545, class 20, object flag 0): no
        // building's task, so LAB_1412796b4's first task of action 3 on class 20
        let ship = Master {
            object_flags: 1,
            garrison: 5,
            ..master(545, 70, class::TRANSPORT, vec![])
        };
        let enter = task(3, class::TRANSPORT, -1, 0.0);
        let villager = master(293, 70, class::VILLAGER, vec![task(3, 3, -1, 0.0), enter]);
        assert_eq!(
            find_with(&villager, &transport(&ship, 2), 3),
            Ok(Some(&enter))
        );
        // any filter takes it too, another not
        assert_eq!(
            find_with(&villager, &transport(&ship, 2), -1),
            Ok(Some(&enter))
        );
        // no task on its class: none with the garrison filter, and none with any filter (the
        // finder ends before the general search, which would give the militia's attack)
        let attack = Task {
            owner_type: 0,
            ..task(7, class::TRANSPORT, -1, 0.0)
        };
        let militia = master(74, 70, 6, vec![attack]);
        assert_eq!(find_with(&militia, &transport(&ship, 2), 3), Ok(None));
        assert_eq!(find_with(&militia, &transport(&ship, 2), -1), Ok(None));
        // an enemy's transport is no garrison: the general search with any filter
        assert_eq!(
            find_with(&militia, &transport(&ship, 1), -1),
            Ok(Some(&attack))
        );
        // a target without object flag 0 has no task with the garrison filter
        let galley = Master {
            object_flags: 0,
            ..ship
        };
        assert_eq!(find_with(&villager, &transport(&galley, 2), 3), Ok(None));
    }

    #[test]
    fn a_villager_garrisons_only_a_transport_s_class_without_the_garrison_filter() {
        // LAB_1412796b4: a villager takes a target with object flag 0 not of class 20 (a ram,
        // class 13) only under the garrison filter (or mode 1, not modelled)
        let ram = Master {
            object_flags: 1,
            garrison: 4,
            ..master(35, 70, class::SIEGE, vec![])
        };
        let enter = task(3, class::SIEGE, -1, 0.0);
        let villager = master(293, 70, class::VILLAGER, vec![enter]);
        assert_eq!(
            find_with(&villager, &transport(&ram, 2), 3),
            Ok(Some(&enter))
        );
        assert_eq!(find_with(&villager, &transport(&ram, 2), -1), Ok(None));
        let infantry = master(74, 70, 6, vec![enter]);
        assert_eq!(
            find_with(&infantry, &transport(&ram, 2), -1),
            Ok(Some(&enter))
        );
        // a dead ram (state 3) is none of its garrisons
        let dead = TaskTarget {
            state: 3,
            ..transport(&ram, 2)
        };
        assert_eq!(find_with(&infantry, &dead, 3), Ok(None));
    }

    #[test]
    fn the_general_search_takes_a_task_for_the_target_s_class_whose_owner_type_fits() {
        let house = master(70, BUILDING, class::BUILDING, vec![]);
        let attack = Task {
            owner_type: 2,
            ..task(7, class::BUILDING, -1, 0.0)
        };
        let militia = master(74, 70, 6, vec![attack]);
        assert_eq!(
            find(&militia, (-1, 0.0), &building(&house, 1, 2)),
            Ok(Some(&attack))
        );
        assert_eq!(find(&militia, (-1, 0.0), &building(&house, 2, 2)), Ok(None));
        let rubble = TaskTarget {
            attackable: false,
            ..building(&house, 1, 2)
        };
        assert_eq!(find(&militia, (-1, 0.0), &rubble), Ok(None));
    }
}
