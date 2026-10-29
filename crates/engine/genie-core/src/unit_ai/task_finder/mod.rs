//! The task finder (`TRIBE_Task_List` slot 2, `FUN_1412784d0`, with the `RGE_Task_List` finder
//! `FUN_1411a8ba0` and its test of a task on a target, slot 4 `FUN_1411a8620`): which task of a
//! master's task list a unit does on a target object. The work command (object slot 37,
//! `FUN_14106ff70`) asks it of the unit's own master and, through the task swap group, of the
//! player's other masters ([`crate::objects::masters::Masters::work_master`]).
//!
//! The finder tries the special tasks first, each for the targets it is about (dropping off what
//! the unit carries, building its player's foundation, garrisoning, repairing, converting, healing
//! and the like), then the general search: a task for the target's combat level, else for its
//! master, else for its class, each the first in list order the task's own test passes, else a
//! task for the terrain under the target's point. The masters' classes and ids it compares against
//! are engine constants; the game stores them encoded (a class as its value − 0xeac, a master id as
//! its id + 0x609c), the port compares the decoded values and names the encoded ones.

use crate::GenieError;
use crate::objects::masters::class;
use crate::objects::masters::master_type::BUILDING;
use crate::objects::masters::{Master, Task};
use crate::unit_ai::task_finder::target_steps::on_target;

mod target_steps;
pub(crate) mod world;

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

/// What the work command's finder searches on: its target (none for a point), the target its
/// general search reads (the main building's) and the terrain under the command's point
/// (`FUN_1411a8ba0`, [`crate::map::terrain::Terrain::at_task_point`]; none off the map).
#[derive(Clone, Copy, Debug)]
pub struct TaskOn<'t, 'a> {
    /// The target, none for a point.
    pub target: Option<&'t TaskTarget<'a>>,
    /// The target the general search reads.
    pub general: Option<&'t TaskTarget<'a>>,
    /// The terrain under the point.
    pub terrain: Option<i16>,
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
    /// `+0x30`, `update_ai_resource_types`: the basic resources (0 to 3) it takes, which the
    /// table's search by resource reads ([`table_drop_buildings`]).
    pub resource_types: Vec<i32>,
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

/// `FUN_140fdd950` with no buildings left out: the buildings of the drop-site table's entries
/// for workers of class `class` (any for −1; an entry's own worker list is not read) that list
/// resource types (`+0x40`), for `civ` where the entry names civs, that take `kind`: a basic
/// resource (below 4, compared unsigned, so −1 is not one) among their resource types, any other
/// among their targets' attribute types; in table order.
#[must_use]
pub fn table_drop_buildings(table: &[DropSite], kind: i16, class: i16, civ: u8) -> Vec<i16> {
    let kind = i32::from(kind);
    table
        .iter()
        .filter(|e| class == -1 || e.worker_class == class)
        .filter(|e| !e.resource_types.is_empty())
        .filter(|e| e.civs.is_empty() || e.civs.contains(&civ))
        .filter(|e| {
            if (kind as u32) < 4 {
                e.resource_types.contains(&kind)
            } else {
                e.targets.iter().any(|t| i32::from(t.attribute) == kind)
            }
        })
        .map(|e| e.building)
        .collect()
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
    if let Finding::Answer(task) = finder_head(list, actor, target, filter, dip)? {
        return Ok(task);
    }
    if let Some(t) = target
        && let Finding::Answer(task) = on_target(list, actor, t, filter, dip)?
    {
        return Ok(task);
    }
    finder_tail(list, actor, (target, general_target), terrain, filter, dip)
}

/// A step of the task finder: it answers (a task, or none), or the finder goes on.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Finding<'m> {
    /// The finder's answer.
    Answer(Option<&'m Task>),
    /// The next step decides.
    Next,
}

/// The task finder's first steps: the garrison filter (3, [`garrison_task`]; with none, a
/// target that is no transport has no task); no task of a master with object flag 0x10 on prey
/// that stores something; the relic carrier's task 0x87 at its player's monastery (master
/// 109). A monk's conversion under attribute 95 is not modelled.
fn finder_head<'m>(
    list: &'m Master,
    actor: &Actor<'_>,
    target: Option<&TaskTarget<'_>>,
    filter: i16,
    dip: &impl Diplomacy,
) -> Result<Finding<'m>, GenieError> {
    let own = actor.master;
    // the garrison filter (3): a building's garrison task; with none, a target whose master has
    // object flag 0 (a transport) goes on through the finder to its garrison into the target
    // (`LAB_1412796b4`), any other has no task
    if filter == 3 {
        if let Some(task) = garrison_task(list, actor, target, dip)? {
            return Ok(Finding::Answer(Some(task)));
        }
        if target.is_none_or(|t| t.master.object_flags & 1 == 0) {
            return Ok(Finding::Answer(None));
        }
    }
    // a master with object flag 0x10 has no task on prey whose master stores something
    // (`+0x610`, its first storage amount, above 0)
    if let Some(t) = target
        && own.object_flags & 0x10 != 0
        && t.master.class == class::PREY
        && 0.0
            < t.master
                .storage
                .first()
                .map_or(0.0, |&(_, amount, _)| amount)
    {
        return Ok(Finding::Answer(None));
    }
    // LAB_141278e9f: the monk tasks for carried relics and conversions of villagers
    let (held_kind, _) = actor.held;
    if held_kind == 0x39
        && target
            .is_some_and(|t| t.state == 2 && t.player == actor.player && t.master.id == id::ID_109)
        && let Some(task) = first(list, |task| task.action_type == 0x87)
    {
        return Ok(Finding::Answer(Some(task)));
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
    Ok(Finding::Next)
}

/// The task finder's tail (`LAB_14127ba9e`, `LAB_14127bb9f`): task 13 for an own, allied or
/// artifact target below state 3 (under that filter alone); a villager on livestock, or on
/// what is inside something, takes task 110; anything else the general search
/// ([`general`]). Conversions, task 0x84, healing, task 0x88, class 53's conversion and master
/// 1105 are not modelled.
fn finder_tail<'m>(
    list: &'m Master,
    actor: &Actor<'_>,
    (target, general_target): (Option<&TaskTarget<'_>>, Option<&TaskTarget<'_>>),
    terrain: Option<i16>,
    filter: i16,
    dip: &impl Diplomacy,
) -> Result<Option<&'m Task>, GenieError> {
    let own = actor.master;
    // the tail: conversions, task 0x84, healing, task 0x88, the siege and class-53 tasks
    let monk = own.class == class::MONK;
    let converts = list.tasks.iter().any(|t| t.action_type == 0x68);
    if target.is_some() && (monk || (converts && filter == 0x68)) {
        return Err(GenieError::unsupported(
            "the task finder's conversion (0x68)",
        ));
    }
    if list.tasks.iter().any(|t| t.action_type == 0x84)
        && target.is_some_and(|t| t.master.class == class::RELIC)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::unit_ai::task_finder::target_steps::{
        drop_off_finding, garrison_into, repair_finding,
    };

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

    /// Players 1 and 3 are allies both ways; player 1 counts 4 an ally, 4 does not count 1.
    struct Teams;

    impl Diplomacy for Teams {
        fn allied(&self, from: usize, to: usize, _: bool) -> bool {
            from == to || matches!((from, to), (1 | 3, 3 | 1) | (1, 4))
        }

        fn drops_at(&self, _: i16, _: &Actor<'_>) -> bool {
            false
        }
    }

    /// `FUN_1411a8620`, a task's own test on a target: a holder's task wants the target to hold
    /// some of its resource (either of its two, or any for a task of neither); types 3 and 106
    /// never apply, attacks (7, 9) want an attackable target; then the owner type says whose
    /// target, and the unit itself is never its own target but for type 4.
    #[test]
    fn a_task_applies_by_what_the_target_holds_and_whose_it_is() {
        let own = Master {
            combat_level: 0,
            ..Master::default()
        };
        let fighter = Master {
            combat_level: 1,
            ..Master::default()
        };
        let me = |master| Actor {
            id: 9181,
            player: 1,
            ..actor(master, (-1, 0.0))
        };
        let of = |owner_type| Task {
            owner_type,
            ..Task::default()
        };
        let at = |player, held| TaskTarget {
            held,
            ..building(&own, player, 2)
        };
        let applies_to = |task: &Task, master, target: &TaskTarget<'_>| {
            applies(task, &me(master), target, &Teams)
        };
        // the holder's test
        let holder = |attribute_type, attribute_type3| Task {
            holding_attr: 1,
            attribute_type,
            attribute_type3,
            ..of(0)
        };
        for (task, held, takes) in [
            (holder(5, -1), (-1, 3.0), false),
            (holder(5, -1), (5, 0.0), false),
            (holder(5, -1), (6, 3.0), false),
            (holder(5, -1), (5, 3.0), true),
            (holder(5, 6), (6, 3.0), true),
            (holder(-1, -1), (9, 3.0), true),
        ] {
            assert_eq!(applies_to(&task, &own, &at(1, held)), takes, "{held:?}");
        }
        // the types that never apply, and the attacks on what cannot be attacked
        for (action_type, attackable, takes) in [
            (3, true, false),
            (106, true, false),
            (7, false, false),
            (9, false, false),
            (9, true, true),
        ] {
            let task = Task {
                action_type,
                ..of(0)
            };
            let target = TaskTarget {
                attackable,
                ..at(1, (-1, 0.0))
            };
            assert_eq!(applies_to(&task, &own, &target), takes, "{action_type}");
        }
        // the owner types, by the target's player
        for (owner_type, player, master, takes) in [
            (1, 1, &own, true),
            (1, 2, &own, false),
            (2, 2, &own, true),
            (2, 0, &own, false),
            (2, 3, &own, false),
            (2, 1, &own, false),
            (3, 0, &own, true),
            (3, 2, &own, false),
            (4, 0, &own, true),
            (4, 3, &own, true),
            (4, 4, &own, false),
            (4, 2, &own, false),
            (5, 1, &own, false),
            (5, 3, &own, false),
            (5, 2, &own, true),
            (5, 0, &own, false),
            (5, 0, &fighter, true),
            (6, 2, &own, true),
            (6, 1, &own, false),
            (0, 1, &own, true),
        ] {
            let target = at(player, (-1, 0.0));
            let got = applies_to(&of(owner_type), master, &target);
            assert_eq!(got, takes, "type {owner_type} on player {player}");
        }
        // the unit itself: only type 4 takes it
        let itself = TaskTarget {
            id: 9181,
            ..at(1, (-1, 0.0))
        };
        assert!(applies_to(&of(4), &own, &itself));
        assert!(!applies_to(&of(1), &own, &itself));
    }

    fn find<'m>(
        list: &'m Master,
        held: (i16, f32),
        t: &TaskTarget<'_>,
    ) -> Result<Option<&'m Task>, GenieError> {
        find_task(list, &actor(list, held), Some(t), Some(t), None, -1, &Alone)
    }

    /// `FUN_140fdd950`: an entry for the worker class with resource types, for the civ, whose
    /// basic resources or targets take the kind.
    #[test]
    fn the_table_gives_the_buildings_that_take_a_kind_from_a_class() {
        let entry = |building, class, types: Vec<i32>, civs: Vec<u8>, attribute| DropSite {
            building,
            worker_class: class,
            civs,
            resource_types: types,
            targets: vec![DropTarget {
                class: 15,
                workers: -1,
                state: -1,
                attribute,
            }],
            ..DropSite::default()
        };
        let table = [
            entry(109, 4, vec![0, 1, 2, 3], vec![], 15),
            // no resource types: never listed
            entry(68, 4, vec![], vec![], 1),
            entry(562, 4, vec![1], vec![], 1),
            // another civ's
            entry(1754, 4, vec![1], vec![17], 1),
            entry(45, 13, vec![0], vec![], 17),
        ];
        assert_eq!(table_drop_buildings(&table, 1, 4, 1), [109, 562]);
        assert_eq!(table_drop_buildings(&table, 1, 4, 17), [109, 562, 1754]);
        // a kind of 4 or more by the targets' attribute types; −1 is not a basic resource
        assert_eq!(table_drop_buildings(&table, 15, 4, 1), [109]);
        assert_eq!(table_drop_buildings(&table, 17, -1, 1), [45]);
        assert_eq!(table_drop_buildings(&table, -1, 4, 1), [0i16; 0]);
    }

    /// `FUN_1412784d0` for a master with object flag 0x10: no task on prey (class 9) whose master
    /// stores something (`+0x610` above 0); prey without storage takes its task as any target.
    #[test]
    fn a_flag_0x10_master_has_no_task_on_prey_that_stores_food() {
        let hunt = task(110, class::PREY, -1, 1.0);
        let hunter = Master {
            object_flags: 0x10,
            ..master(122, 70, class::VILLAGER, vec![hunt])
        };
        let deer = Master {
            storage: vec![(0, 100.0, 1)],
            ..master(65, 70, class::PREY, vec![])
        };
        let target = |m| TaskTarget {
            state: 2,
            ..building(m, 0, 2)
        };
        assert_eq!(find(&hunter, (-1, 0.0), &target(&deer)), Ok(None));
        let eaten = Master {
            storage: vec![(0, 0.0, 1)],
            ..deer.clone()
        };
        assert_eq!(find(&hunter, (-1, 0.0), &target(&eaten)), Ok(Some(&hunt)));
        let plain = Master {
            object_flags: 0,
            ..hunter
        };
        assert_eq!(find(&plain, (-1, 0.0), &target(&deer)), Ok(Some(&hunt)));
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

    /// The repair branch (`LAB_1412796b4`): a damaged building of the player's own (or Gaia's)
    /// takes the repair task for its master first, else the last for its class with a work value,
    /// else the last for any target; under the repair filter with none the finder ends; a whole
    /// building or another player's has none.
    #[test]
    fn a_damaged_building_takes_the_repair_task_for_its_master_or_class() {
        let house = master(70, BUILDING, class::BUILDING, vec![]);
        let damaged = TaskTarget {
            hp: 300.0,
            ..building(&house, 2, 2)
        };
        let by = |tasks: Vec<Task>, t: &TaskTarget<'_>, filter: i16| {
            let builder = master(118, 70, class::VILLAGER, tasks);
            find_with(&builder, t, filter).map(Option::<&Task>::copied)
        };
        let any = task(106, -1, -1, 0.0);
        let class3 = task(106, 3, -1, 2.0);
        let own = task(106, -1, 70, 0.0);
        assert_eq!(by(vec![any], &damaged, -1), Ok(Some(any)));
        assert_eq!(by(vec![class3, any], &damaged, -1), Ok(Some(class3)));
        assert_eq!(by(vec![class3, any, own], &damaged, -1), Ok(Some(own)));
        // a class task without a work value leaves no task, not the any one
        assert_eq!(
            by(vec![task(106, 3, -1, 0.0), any], &damaged, 106),
            Ok(None)
        );
        // Gaia's damaged building too; whole or another player's, none
        let gaia = TaskTarget {
            player: 0,
            ..damaged
        };
        assert_eq!(by(vec![any], &gaia, 106), Ok(Some(any)));
        assert_eq!(by(vec![any], &building(&house, 2, 2), 106), Ok(None));
        let foreign = TaskTarget {
            player: 3,
            ..damaged
        };
        assert_eq!(by(vec![any], &foreign, 106), Ok(None));
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
    /// The finder's head: the garrison filter without a garrison task answers none on a target
    /// that is no transport and goes on for a transport; a flag-0x10 master has none on prey
    /// that stores something; the relic carrier takes task 0x87 at its monastery (master 109);
    /// a monk's conversion under attribute 95 is not modelled; anything else goes on.
    #[test]
    fn the_finder_s_head_answers_the_garrison_filter_prey_and_relics() {
        let house = master(70, BUILDING, class::BUILDING, vec![]);
        let villager = master(83, 70, class::VILLAGER, vec![task(0x87, -1, -1, 0.0)]);
        let a = actor(&villager, (-1, 0.0));
        let t = building(&house, 2, 2);
        assert_eq!(
            finder_head(&villager, &a, Some(&t), -1, &Alone),
            Ok(Finding::Next)
        );
        assert_eq!(
            finder_head(&villager, &a, Some(&t), 3, &Alone),
            Ok(Finding::Answer(None))
        );
        let ship = Master {
            object_flags: 1,
            ..master(545, 70, class::TRANSPORT, vec![])
        };
        assert_eq!(
            finder_head(&villager, &a, Some(&transport(&ship, 2)), 3, &Alone),
            Ok(Finding::Next)
        );
        let deer = Master {
            storage: vec![(0, 100.0, 0)],
            ..master(65, 70, class::PREY, vec![])
        };
        let wolf = Master {
            object_flags: 0x10,
            ..master(126, 70, 10, vec![])
        };
        let prey = building(&deer, 0, 2);
        assert_eq!(
            finder_head(&wolf, &actor(&wolf, (-1, 0.0)), Some(&prey), -1, &Alone),
            Ok(Finding::Answer(None))
        );
        let monastery = master(id::ID_109, BUILDING, class::BUILDING, vec![]);
        let carrier = actor(&villager, (0x39, 1.0));
        assert_eq!(
            finder_head(
                &villager,
                &carrier,
                Some(&building(&monastery, 2, 2)),
                -1,
                &Alone
            ),
            Ok(Finding::Answer(Some(&villager.tasks[0])))
        );
        let monk = master(id::ID_448, 70, class::MONK, vec![]);
        let converting = Actor {
            attribute_95: 2.0,
            ..actor(&monk, (-1, 0.0))
        };
        let peasant = building(&villager, 3, 2);
        assert!(finder_head(&monk, &converting, Some(&peasant), -1, &Alone).is_err());
    }

    /// The general search's last step (`FUN_1411a8ba0`): without a task for the target, or
    /// without a target, the first task for the terrain under the point (the transport's beach
    /// task, 12 on terrain 2), whatever the filter the finder's tail passes (−1); none without a
    /// terrain (a point off the map, or x or y −1).
    #[test]
    fn a_point_on_a_terrain_takes_the_task_for_that_terrain() {
        let beach = Task {
            terrain: 2,
            ..task(12, -1, -1, 0.0)
        };
        let ship = master(
            545,
            70,
            class::TRANSPORT,
            vec![task(109, -1, -1, 0.0), beach],
        );
        let a = actor(&ship, (-1, 0.0));
        let found = |t: Option<&TaskTarget<'_>>, terrain, filter| {
            find_task(&ship, &a, t, t, terrain, filter, &Alone).map(|f| f.map(|k| k.action_type))
        };
        assert_eq!(found(None, Some(2), -1), Ok(Some(12)));
        assert_eq!(found(None, Some(2), 12), Ok(Some(12)));
        assert_eq!(found(None, Some(1), -1), Ok(None));
        assert_eq!(found(None, None, -1), Ok(None));
        let villager = master(83, 70, class::VILLAGER, vec![]);
        let peasant = building(&villager, 2, 2);
        assert_eq!(found(Some(&peasant), Some(2), -1), Ok(Some(12)));
    }

    /// The finder's steps on a target: its player's foundation takes the build task; master 199
    /// is not modelled; a dock or market below state 3 takes the first trade task (0x6f) for its
    /// master or class the filter takes, and without one goes on; a transport's garrison without
    /// a task ends the finder (none); anything else goes on.
    #[test]
    fn the_finder_s_steps_on_a_target_answer_or_go_on() {
        let house = master(70, BUILDING, class::BUILDING, vec![]);
        let builder = master(118, 70, class::VILLAGER, vec![task(101, -1, -1, 1.0)]);
        let a = actor(&builder, (-1, 0.0));
        assert_eq!(
            on_target(&builder, &a, &building(&house, 2, 0), -1, &Alone),
            Ok(Finding::Answer(Some(&builder.tasks[0])))
        );
        assert_eq!(
            on_target(&builder, &a, &building(&house, 2, 2), -1, &Alone),
            Ok(Finding::Next)
        );
        let m199 = master(id::ID_199, BUILDING, class::BUILDING, vec![]);
        assert!(on_target(&builder, &a, &building(&m199, 2, 2), -1, &Alone).is_err());
        let m45 = master(id::ID_45, 70, 6, vec![]);
        assert_eq!(
            on_target(&builder, &a, &building(&m45, 2, 2), -1, &Alone),
            Ok(Finding::Next)
        );
        let trader = master(
            128,
            70,
            19,
            vec![
                task(0x6f, 7, -1, 1.0),
                task(0x6f, -1, id::ID_84, 1.0),
                task(0x6f, 6, -1, 1.0),
            ],
        );
        let cart = actor(&trader, (-1, 0.0));
        let dock = building(&m45, 2, 2);
        assert_eq!(
            on_target(&trader, &cart, &dock, -1, &Alone),
            Ok(Finding::Answer(Some(&trader.tasks[2])))
        );
        assert_eq!(
            on_target(&trader, &cart, &dock, 0x6f, &Alone),
            Ok(Finding::Answer(Some(&trader.tasks[2])))
        );
        assert_eq!(
            on_target(&trader, &cart, &dock, 5, &Alone),
            Ok(Finding::Next)
        );
        let market = master(id::ID_84, BUILDING, 7, vec![]);
        assert_eq!(
            on_target(&trader, &cart, &building(&market, 2, 2), -1, &Alone),
            Ok(Finding::Answer(Some(&trader.tasks[0])))
        );
        // state 3 and up: no trade
        assert_eq!(
            on_target(&trader, &cart, &building(&m45, 2, 3), -1, &Alone),
            Ok(Finding::Next)
        );
        let soldier = master(4, 70, 6, vec![]);
        let ship = Master {
            object_flags: 1,
            ..master(545, 70, class::TRANSPORT, vec![])
        };
        let sailor = actor(&soldier, (-1, 0.0));
        assert_eq!(
            on_target(&soldier, &sailor, &transport(&ship, 2), -1, &Alone),
            Ok(Finding::Answer(None))
        );
    }

    /// The drop-off step: a lumberjack carrying wood to its finished camp takes its chop task;
    /// carrying nothing goes on; without a task for the load, a camp at another work answers
    /// none, an idle one goes on.
    #[test]
    fn the_drop_off_step_takes_the_task_for_the_load() {
        fn drop<'m>(list: &'m Master, held: (i16, f32), t: &TaskTarget<'_>) -> Finding<'m> {
            drop_off_finding(list, &actor(list, held), t, -1, &Alone)
        }
        let camp = master(562, BUILDING, class::BUILDING, vec![]);
        let chop = Task {
            attribute_type: 0,
            ..task(5, 15, -1, 0.6)
        };
        let lumberjack = master(123, 70, class::VILLAGER, vec![chop]);
        let site = building(&camp, 2, 2);
        assert_eq!(
            drop(&lumberjack, (0, 10.0), &site),
            Finding::Answer(Some(&lumberjack.tasks[0]))
        );
        assert_eq!(drop(&lumberjack, (-1, 0.0), &site), Finding::Next);
        let idle = master(123, 70, class::VILLAGER, vec![]);
        assert_eq!(drop(&idle, (0, 10.0), &site), Finding::Next);
        let busy = TaskTarget {
            top_action: Some(0x10),
            ..site
        };
        assert_eq!(drop(&idle, (0, 10.0), &busy), Finding::Answer(None));
    }

    /// The garrison into a transport: its class's task of action 3; without one the garrison
    /// filter has none and any filter ends the finder after the repair; a villager goes on for
    /// a transport of another class.
    #[test]
    fn the_garrison_into_a_transport_takes_its_class_s_task() {
        fn into<'m>(list: &'m Master, t: &TaskTarget<'_>, filter: i16) -> (Finding<'m>, bool) {
            garrison_into(list, &actor(list, (-1, 0.0)), t, filter, &Alone)
        }
        let ship = Master {
            object_flags: 1,
            ..master(545, 70, class::TRANSPORT, vec![])
        };
        let boarding = task(crate::orders::GARRISON, class::TRANSPORT, -1, 0.0);
        let soldier = master(4, 70, 6, vec![boarding]);
        let bare = master(4, 70, 6, vec![]);
        let t = transport(&ship, 2);
        assert_eq!(
            into(&soldier, &t, -1),
            (Finding::Answer(Some(&soldier.tasks[0])), false)
        );
        assert_eq!(into(&bare, &t, 3), (Finding::Answer(None), false));
        assert_eq!(into(&bare, &t, -1), (Finding::Next, true));
        let barge = Master {
            object_flags: 1,
            ..master(546, 70, 6, vec![])
        };
        let villager = master(83, 70, class::VILLAGER, vec![]);
        assert_eq!(
            into(&villager, &transport(&barge, 2), -1),
            (Finding::Next, false)
        );
    }

    /// The repair step: a damaged building takes the repair task for any target; a whole one
    /// goes on; under the repair filter a class's task without a work value leaves none.
    #[test]
    fn the_repair_step_takes_a_damaged_building_s_task() {
        fn repair<'m>(list: &'m Master, t: &TaskTarget<'_>, filter: i16) -> Finding<'m> {
            repair_finding(list, &actor(list, (-1, 0.0)), t, filter, &Alone)
        }
        let house = master(70, BUILDING, class::BUILDING, vec![]);
        let damaged = TaskTarget {
            hp: 300.0,
            ..building(&house, 2, 2)
        };
        let any = master(118, 70, class::VILLAGER, vec![task(106, -1, -1, 0.0)]);
        assert_eq!(
            repair(&any, &damaged, -1),
            Finding::Answer(Some(&any.tasks[0]))
        );
        assert_eq!(repair(&any, &building(&house, 2, 2), -1), Finding::Next);
        let weak = master(
            118,
            70,
            class::VILLAGER,
            vec![task(106, class::BUILDING, -1, 0.0)],
        );
        assert_eq!(repair(&weak, &damaged, 106), Finding::Answer(None));
        assert_eq!(repair(&weak, &damaged, -1), Finding::Next);
    }

    /// The finder's tail: task 13 under its filter alone for an own target; a villager on
    /// livestock takes task 110 for class 58; a monk's conversion, task 0x84, healing, task
    /// 0x88, class 53's conversion and master 1105 are not modelled.
    #[test]
    fn the_finder_s_tail_takes_task_13_livestock_or_the_general_search() {
        fn tail<'m>(
            list: &'m Master,
            t: &TaskTarget<'_>,
            filter: i16,
        ) -> Result<Option<&'m Task>, GenieError> {
            finder_tail(
                list,
                &actor(list, (-1, 0.0)),
                (Some(t), Some(t)),
                None,
                filter,
                &Alone,
            )
        }
        let house = master(70, BUILDING, class::BUILDING, vec![]);
        let own = building(&house, 2, 2);
        let thirteen = master(9, 70, 6, vec![task(0xd, -1, -1, 0.0)]);
        assert_eq!(tail(&thirteen, &own, 0xd), Ok(Some(&thirteen.tasks[0])));
        assert_eq!(tail(&thirteen, &own, -1), Ok(None));
        let sheep = master(594, 70, class::LIVESTOCK, vec![]);
        let shepherd = master(
            83,
            70,
            class::VILLAGER,
            vec![task(0x6e, class::LIVESTOCK, -1, 0.0)],
        );
        assert_eq!(
            tail(&shepherd, &building(&sheep, 0, 2), -1),
            Ok(Some(&shepherd.tasks[0]))
        );
        for list in [
            master(125, 70, class::MONK, vec![]),
            master(9, 70, 6, vec![task(0x69, -1, -1, 0.0)]),
            master(9, 70, class::CLASS_43, vec![]),
            master(9, 70, class::CLASS_53, vec![]),
            master(id::ID_1105, 70, class::SIEGE, vec![]),
        ] {
            assert!(tail(&list, &own, -1).is_err(), "{}", list.id);
        }
        let relic_cart = master(9, 70, 6, vec![task(0x84, -1, -1, 0.0)]);
        let c42 = master(1, 70, class::RELIC, vec![]);
        assert!(tail(&relic_cart, &building(&c42, 2, 2), -1).is_err());
    }
}
