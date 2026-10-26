//! A building's production: the queue QUEUE and RESEARCH fill (`TRIBE_Building_Object`
//! `+0xbe8` to `+0xc01`, `+0xcd8`) and the action of its production list (`+0xbf8`) that works
//! the first entry off: the make action (`TRIBE_Action_Make_Obj`, update `0x140ad0490`) or the
//! research's (`TRIBE_Action_Make_Tech`). The building's update makes and runs that action after
//! its combat update (`FUN_1411e7dc0`, [`crate::production::spawn`]); QUEUE's building part is
//! `FUN_1411cbc40` ([`crate::turn`]); the cancels are `FUN_1411d66b0`, `FUN_1411d61e0` and
//! `FUN_1411cff00` ([`crate::players::game_status`]).
//!
//! The queue is a buffer of 0x30-byte slots (`+0xbe8`, room for `+0xbf0`), the first `+0xbf2`
//! of them its entries: a master or a technology and how many of it. An entry that empties goes:
//! the entries after it move down a slot, field by field (the serial `+0x20` stays where it was),
//! and the slot past the new last one keeps what it held. The slots past the count are never
//! cleared: a field a new entry does not write (a research's train time) keeps an old entry's
//! value, and the buffer's regrowth by ten copies them with the rest.

use crate::GenieError;
use crate::objects::masters::{Master, Masters, TrainLocation};

pub mod spawn;

/// One slot of a building's queue (`+0xbe8`, 0x30 bytes; the save's `mProductionQueue[i]`). A
/// slot the buffer's growth adds is zeroed (`calloc`), as [`Entry::default`] is.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Entry {
    /// `+0x0`, `mMasterUnitId`: the master it makes; −1 for a research.
    pub master: i16,
    /// `+0x2`, `mUnitCount`: how many.
    pub count: i16,
    /// `+0x4`, `mTechId`: the technology it researches; −1 for a unit.
    pub tech: i16,
    /// `+0x8`, `mCosts`: what each of its units or researches paid, three (attribute, amount)
    /// pairs (`FUN_1411c3190`), which a cancel gives back.
    pub costs: [(i32, f32); 3],
    /// `+0x20`: the building's serial (`+0xcd8`) when the entry came; not saved (a load numbers
    /// the slots it reads from 0).
    pub serial: i64,
    /// `+0x28`, `mButtonLocationId`: the train button QUEUE named, −1 for none.
    pub button: i32,
    /// `+0x2c`, `mTrainTime`: the unit's train time at the building and button
    /// ([`train_time`]); a research entry does not write it.
    pub train_time: i16,
}

/// A queue entry's costs before `FUN_1411c3190` fills them: no attribute, nothing (`0xffffffff`
/// in each pair's eight bytes).
pub const NO_COSTS: [(i32, f32); 3] = [(-1, 0.0); 3];

/// How many slots the buffer grows by when it is full (`FUN_1411cbc40`).
pub const GROWTH: i16 = 10;

/// The units and researches a building holds queued at most (`+0xbf4`, `FUN_1411cbc40`).
pub const ROOM: i16 = 15;

/// The queue of a building.
#[derive(Clone, Debug, PartialEq)]
pub struct Queue {
    /// `+0xbe8`, its length `+0xbf0`: the buffer's slots, the entries first.
    pub slots: Vec<Entry>,
    /// `+0xbf2`, `mProductionQueueCount`: how many slots are entries.
    pub count: i16,
    /// `+0xbf4`, `mProductionQueueTotalUnits`: the entries' counts summed.
    pub total: i16,
    /// `+0xbf6`, `mProductionQueueEnabled`: the building's update works the queue; a new
    /// building's constructor sets it, and nothing in the simulation clears it.
    pub enabled: bool,
    /// `+0xc00`: grows by one with every change of the entries (a byte, wrapping).
    pub changes: u8,
    /// `+0xc01`: the last update made nothing because the building's own top action is a make,
    /// a research or of types 0x7b to 0x7d (`FUN_141227a00`) for a player not of type 3.
    pub waiting: bool,
    /// `+0xcd8`: the serial the next entry takes (not saved; a load leaves it at the slots it
    /// read).
    pub next_serial: i64,
}

impl Default for Queue {
    /// A new building's queue: no slots, enabled.
    fn default() -> Self {
        Self {
            slots: Vec::new(),
            count: 0,
            total: 0,
            enabled: true,
            changes: 0,
            waiting: false,
            next_serial: 0,
        }
    }
}

/// Two entries' costs are the same (`FUN_1411cbc40`'s join): each pair's attribute as an
/// integer and its amount as a float (`UCOMISS`, `JP`: a NaN differs).
#[must_use]
pub fn same_costs(a: &[(i32, f32); 3], b: &[(i32, f32); 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| x.0 == y.0 && x.1 == y.1)
}

impl Queue {
    /// The entries (the first `+0xbf2` slots).
    #[must_use]
    pub fn entries(&self) -> &[Entry] {
        let n = usize::try_from(self.count)
            .unwrap_or(0)
            .min(self.slots.len());
        &self.slots[..n]
    }

    /// The first entry, if any.
    #[must_use]
    pub fn first(&self) -> Option<&Entry> {
        self.entries().first()
    }

    /// The last entry, if any.
    #[must_use]
    pub fn last(&self) -> Option<&Entry> {
        self.entries().last()
    }

    /// `FUN_1411d66b0`'s walk: the entry of the `index`-th unit, counting each entry's units in
    /// turn; None for an empty queue or an index past the last unit.
    #[must_use]
    pub fn entry_of_unit(&self, index: u32) -> Option<usize> {
        let entries = self.entries();
        if entries.is_empty() {
            return None;
        }
        let (mut e, mut k) = (0usize, 0i32);
        for _ in 0..index {
            k += 1;
            if i32::from(entries[e].count) <= k {
                e += 1;
                k = 0;
                if entries.len() <= e {
                    return None;
                }
            }
        }
        Some(e)
    }

    /// An emptied entry `i` goes (its caller lowered `+0xbf2` first in the game; here this
    /// does): each later entry moves down a slot, its master, count, technology, costs and
    /// button, and its train time when `time` (`FUN_1411d61e0`'s research branch does not move
    /// it); the serials stay, and the slot of the old last entry keeps what it held.
    pub fn close(&mut self, i: usize, time: bool) {
        self.count -= 1;
        let mut j = i;
        while (j as i32) < i32::from(self.count) {
            let next = self.slots[j + 1];
            let slot = &mut self.slots[j];
            slot.master = next.master;
            slot.count = next.count;
            slot.tech = next.tech;
            slot.costs = next.costs;
            slot.button = next.button;
            if time {
                slot.train_time = next.train_time;
            }
            j += 1;
        }
    }

    /// `FUN_1411cbc40`'s growth of a full buffer (`+0xbf0` at most `+0xbf2`): ten zeroed slots
    /// more; each old slot keeps its master, count, technology, button, train time and serial,
    /// and takes its costs from `costs`, one per old slot (`FUN_1411c3190` filled them anew from
    /// (−1, 0) with the slot's technology and master, as they stand now: [`Queue::full`]).
    pub fn grow(&mut self, costs: &[[(i32, f32); 3]]) {
        let old = self.slots.len();
        let mut slots = vec![Entry::default(); old + GROWTH as usize];
        for ((new, slot), &costs) in slots.iter_mut().zip(&self.slots).zip(costs) {
            *new = Entry { costs, ..*slot };
        }
        self.slots = slots;
    }

    /// The buffer is full: a new entry grows it first (`+0xbf0` at most `+0xbf2`).
    #[must_use]
    pub fn full(&self) -> bool {
        self.slots.len() as i32 <= i32::from(self.count)
    }

    /// `FUN_1411cbc40`'s new entry in the slot after the last (the buffer grown first): its
    /// master, count, technology, button and serial (`+0xcd8`, which grows), and `costs`; its
    /// train time is the caller's. The count, the total and the change counter are the
    /// caller's too. Returns the slot.
    pub fn open(&mut self, master: i16, count: i16, tech: i16, button: i32) -> usize {
        let i = usize::try_from(self.count).unwrap_or(0);
        let serial = self.next_serial;
        self.next_serial += 1;
        let slot = &mut self.slots[i];
        slot.master = master;
        slot.count = count;
        slot.tech = tech;
        slot.button = button;
        slot.serial = serial;
        slot.costs = NO_COSTS;
        i
    }

    /// `FUN_1411e7dc0`'s release of the first entry's unit or research once its action is done
    /// or failed: its count, the total and the change counter; an emptied first entry goes
    /// ([`Queue::close`], the train times moved).
    pub fn release_first(&mut self) {
        if self.count == 0 {
            return;
        }
        let first = &mut self.slots[0];
        first.count -= 1;
        self.total -= 1;
        self.changes = self.changes.wrapping_add(1);
        if self.slots[0].count < 1 {
            self.close(0, true);
        }
    }
}

/// `FUN_1411cbc40`'s train time for a unit of a master with train locations `locations`
/// queued at a building of master `building` with button `button` (and `FUN_140acf280`'s for a
/// master without flag 0x80): with button −1 the first location's time; else the location of
/// that building whose button is the button's low byte, else the first location's; 0 without
/// locations.
#[must_use]
pub fn train_time(locations: &[TrainLocation], building: i16, button: i32) -> i16 {
    let first = locations.first().map_or(0, |l| l.time);
    if button == -1 {
        return first;
    }
    locations
        .iter()
        .find(|l| l.location == building && l.button == button as u8)
        .map_or(first, |l| l.time)
}

/// `object_flags` (`+0x654`) bit 7: the make action takes the train time its queue entry
/// recorded (`FUN_140acf280`).
const OWN_TRAIN_TIME: u32 = 0x80;

/// `TRIBE_Action_Make_Obj` (type 0x66, 0x80 bytes): the make action of a building's first
/// queue entry (the save's `tribe_action_make_obj`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MakeObj {
    /// `+0x18`: its state (6 while it works, 1 once its unit came; 0 when its start found no
    /// master, `FUN_140acf0a0`).
    pub state: u8,
    /// `+0x60`: the queue paid for its unit (`FUN_140acb140`'s `param_5` 0), so its destructor
    /// gives nothing back; clear, the destructor refunds the master's paid costs
    /// (`FUN_141630770`).
    pub paid: bool,
    /// `+0x68`, `obj_id`: the master it makes.
    pub master: i16,
    /// `+0x6c`, `work_done`: the seconds of work done.
    pub progress: f32,
    /// `+0x70`, `unique_id` (−1 from the building's update).
    pub unique_id: i32,
    /// `+0x78`, `mButtonLocationId`.
    pub button: i32,
    /// `+0x7c`, `mTrainTime`: the train time of the entry it was made for.
    pub train_time: i16,
}

impl MakeObj {
    /// The make action's state while it works.
    pub const MAKING: u8 = 6;
    /// Its state once its unit came.
    pub const DONE: u8 = 1;

    /// The building's update's make action (`FUN_140acb140` with the first entry's master, −1,
    /// 0, its train time and button): paid, nothing done, state 0 until the list starts it.
    #[must_use]
    pub const fn for_entry(entry: &Entry) -> Self {
        Self {
            state: 0,
            paid: true,
            master: entry.master,
            progress: 0.0,
            unique_id: -1,
            button: entry.button,
            train_time: entry.train_time,
        }
    }

    /// `FUN_140acf280`: the seconds its unit takes. A master with flag 0x80 (`+0x654`) takes
    /// the entry's train time; any other its train location at the building of master
    /// `building` and the action's button ([`train_time`]); 0 for a master that is not a
    /// combat master (`TRIBE_Master_Combat_Object`, type 70 and up).
    #[must_use]
    pub fn time(&self, master: Option<&Master>, building: i16) -> i16 {
        let Some(m) = master.filter(|m| COMBAT_MASTER <= m.kind) else {
            return 0;
        };
        if m.object_flags & OWN_TRAIN_TIME != 0 {
            return self.train_time;
        }
        train_time(&m.train_locations, building, self.button)
    }
}

/// The first master type that is a `TRIBE_Master_Combat_Object` (the dynamic casts of
/// `FUN_1411cbc40` and `FUN_140acf280`).
pub const COMBAT_MASTER: u8 = 70;

/// `TRIBE_Action_Make_Tech` (type 0x67, 0x78 bytes): the research of a building's first queue
/// entry (the save's `tribe_action_make_tech`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MakeTech {
    /// `+0x18`: its state (6 while it works).
    pub state: u8,
    /// `+0x60`: `FUN_140ad0700`'s `param_5` flipped (the building's update passes 0): set, its
    /// destructor gives nothing back (`FUN_1413ccc00` runs only while it is clear).
    pub paid: bool,
    /// `+0x68`, `tech_id`.
    pub tech: i16,
    /// `+0x6c`, `unique_id`: the entry's master (−1).
    pub master: i32,
    /// `+0x70`, `m_researchProgress`: the work done.
    pub progress: f32,
}

/// What a building's production list (`+0xbf8`) holds: the building's update makes at most one
/// action in it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Making {
    /// A unit's make action.
    Unit(MakeObj),
    /// A research.
    Research(MakeTech),
}

impl Making {
    /// Its state (`+0x18`).
    #[must_use]
    pub const fn state(&self) -> u8 {
        match self {
            Self::Unit(m) => m.state,
            Self::Research(r) => r.state,
        }
    }
}

/// A building's queue, its production list's action and its lists of technologies.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Production {
    /// The queue.
    pub queue: Queue,
    /// The production list's action, if it has one.
    pub action: Option<Making>,
    /// `+0xd00`, `localDisabledTechs`: technologies the building may not research again; a
    /// research of the shared kind joins it (`FUN_1411e7dc0`), a cancel of a technology drops
    /// it (`FUN_1411c5250`).
    pub disabled_techs: Vec<i16>,
    /// `+0xd18`, `localResearchedTechs`: the technologies of the shared kind this building
    /// researched (`FUN_1411e7dc0`).
    pub researched_techs: Vec<i16>,
    /// `+0xd30`, `localAvailableUnits`: masters QUEUE takes at this building though they are
    /// not available to the player (`FUN_1411c8fd0`).
    pub available_units: Vec<i16>,
}

impl Production {
    /// The make action's state while it works, which the building's action-state slot reports.
    pub const MAKING: u8 = MakeObj::MAKING;

    /// `FUN_1411c5250`: technology `tech` leaves the building's disabled list, every time it
    /// is there (`std::remove` and `erase`, the order of the rest kept).
    pub fn enable_tech(&mut self, tech: i16) {
        self.disabled_techs.retain(|&t| t != tech);
    }

    /// `FUN_1411e7dc0` after a research of the shared kind: the technology joins the
    /// building's researched list, and its disabled list, each unless it holds it; once the
    /// researched list holds it the disabled one is left alone.
    pub fn note_shared_research(&mut self, tech: i16) {
        if self.researched_techs.contains(&tech) {
            return;
        }
        self.researched_techs.push(tech);
        if !self.disabled_techs.contains(&tech) {
            self.disabled_techs.push(tech);
        }
    }
}

/// The population a master's unit takes (`FUN_14163b0b0`): minus its storage of attribute 4,
/// the population headroom, over the master's three storage entries.
#[must_use]
pub fn population(storage: &[(i16, f32, u8)]) -> f32 {
    storage
        .iter()
        .filter(|&&(kind, _, _)| kind == HEADROOM)
        .fold(0.0, |pop, &(_, amount, _)| pop - amount)
}

/// The population headroom (attribute 4), the population (11), the population cap (32), the
/// units queued (80) and the units in production (81).
pub const HEADROOM: i16 = 4;
/// The population (attribute 11).
pub const POPULATION: usize = 11;
/// The population cap (attribute 32).
pub const POPULATION_CAP: usize = 32;
/// The units queued (attribute 80).
pub const QUEUED: i16 = 80;
/// The units in production (attribute 81).
pub const IN_PRODUCTION: i16 = 81;

/// `FUN_1411e7dc0`'s test before it makes a unit's make action: the population with the units in
/// production and this one stays within the cap (`COMISS`, `JBE`: a NaN passes), and the
/// headroom covers them. A building that fails it waits (housed).
pub fn housed(attributes: &[f32], pop: f32) -> Result<bool, GenieError> {
    let a = |i: usize| attributes.get(i).copied().ok_or(crate::turn::NO_ATTRIBUTE);
    let in_production = a(IN_PRODUCTION as usize)?;
    Ok(a(POPULATION_CAP)? < a(POPULATION)? + in_production + pop
        || (a(HEADROOM as usize)? - in_production) - pop < 0.0)
}

/// `FUN_141630530`: a queued unit's refund. For a master of type 70 and up each of its cost
/// entries with an attribute and a paid flag gives back what the queue entry's `costs` paid
/// of that attribute (the last pair that names it; 0 without one), added straight into the
/// player's attribute (no side effects).
pub fn refund(
    master: &Master,
    costs: &[(i32, f32); 3],
    attributes: &mut dyn crate::technology::Attributes,
) -> Result<(), GenieError> {
    if master.kind < COMBAT_MASTER {
        return Ok(());
    }
    for &(attribute, _, paid) in master.cost.iter().take(3) {
        if attribute < 0 || !paid {
            continue;
        }
        let back = costs
            .iter()
            .rev()
            .find(|c| c.0 != -1 && c.0 == i32::from(attribute))
            .map_or(0.0, |c| c.1);
        let held = attributes.get(attribute)?;
        attributes.set(attribute, held + back)?;
    }
    Ok(())
}

/// `FUN_1413d15e0`'s refund of `count` researches of a queue entry that paid `costs`: each
/// pair with an attribute gives back the count times its amount, straight into the player's
/// attribute. (Its technology's state 1 and work done 0 are the caller's.)
pub fn refund_research(
    costs: &[(i32, f32); 3],
    count: i16,
    attributes: &mut dyn crate::technology::Attributes,
) -> Result<(), GenieError> {
    for &(attribute, amount) in costs {
        // the entry's attributes are the costs' i16 attributes (FUN_1411c3190, the load)
        let Ok(attribute) = i16::try_from(attribute) else {
            continue;
        };
        if attribute < 0 {
            continue;
        }
        let held = attributes.get(attribute)?;
        attributes.set(attribute, held + f32::from(count) * amount)?;
    }
    Ok(())
}

/// `FUN_141630770` with `n` 1, the make action's destructor's refund of a unit its queue did
/// not pay for: each cost entry of a master of type 70 and up with an attribute and a paid
/// flag gives back its amount times `n` (an integer product), straight into the attribute.
pub fn refund_master(
    master: &Master,
    n: i32,
    attributes: &mut dyn crate::technology::Attributes,
) -> Result<(), GenieError> {
    if master.kind < COMBAT_MASTER {
        return Ok(());
    }
    for &(attribute, amount, paid) in master.cost.iter().take(3) {
        if attribute < 0 || !paid {
            continue;
        }
        let held = attributes.get(attribute)?;
        attributes.set(attribute, (i32::from(amount) * n) as f32 + held)?;
    }
    Ok(())
}

/// The research cost modifier (attribute 85, `+0x2a8`), which scales a refunded technology's
/// costs above 0.
const RESEARCH_COST_MODIFIER: i16 = 85;

/// `FUN_1413ccc00` with `param_3` 1 for technology `tech`: in state 2 or 4 it gives back its
/// `.dat` costs (each entry with an attribute and a paid flag: its amount plus the state's
/// adjustment, which the research cost modifier, attribute 85 above 0 and not for an age,
/// scales truncated to 16 bits), straight into the attributes, and is in state 1 with no work
/// done. True when it was; in another state nothing happens.
pub fn refund_tech(
    state: &mut crate::technology::TechState,
    def: &crate::technology::TechDef,
    tech: i16,
    attributes: &mut dyn crate::technology::Attributes,
) -> Result<bool, GenieError> {
    use crate::checkpoint::cvttss2si;
    if !matches!(state.state, 2 | 4) {
        return Ok(false);
    }
    let modifier = if crate::technology::is_age(tech) {
        0.0
    } else {
        attributes.get(RESEARCH_COST_MODIFIER)?
    };
    for (j, c) in def.costs.iter().enumerate() {
        if c.attribute < 0 || c.paid == 0 {
            continue;
        }
        let mut v = (i32::from(state.cost_adj[j]) + i32::from(c.amount)) as f32;
        if 0.0 < modifier {
            v = f32::from(cvttss2si(v * modifier) as i16);
        }
        let held = attributes.get(c.attribute)?;
        attributes.set(c.attribute, held + v)?;
    }
    state.state = 1;
    state.research_done = 0.0;
    Ok(true)
}

/// The buildings' master type (`'P'`), whose annex's costs the cost test reads.
const BUILDING_TYPE: u8 = 80;

/// The cost multiplier `FUN_14162ef60`'s callers pass (`DAT_14330eb8c`, 1.0).
const ONE: f32 = 1.0;

/// A master's three cost entries `(attribute, amount, paid)`, −1 for an entry it lacks.
fn cost_entry(master: &Master, j: usize) -> (i16, i16, bool) {
    master.cost.get(j).copied().unwrap_or((-1, 0, false))
}

/// The master whose costs the cost tests read for player's master `m` (`FUN_141618db0`,
/// `FUN_141614450`): a building (type 80) that is an annex (a head unit, `+0x9b8`, the player
/// has) reads its head's, and the head's stack unit's (`+0x986`) when the player has that one,
/// then only the paid entries (the second value true).
#[must_use]
pub fn costed<'m>(masters: &'m Masters, m: &'m Master) -> (&'m Master, bool) {
    if m.kind != BUILDING_TYPE {
        return (m, false);
    }
    let Some(head) = m
        .building
        .as_ref()
        .filter(|b| b.head_unit != -1)
        .and_then(|b| masters.get(b.head_unit))
    else {
        return (m, false);
    };
    head.building
        .as_ref()
        .filter(|b| b.stack_unit != -1)
        .and_then(|b| masters.get(b.stack_unit))
        .map_or((head, false), |stack| (stack, true))
}

/// `FUN_141618db0` without a reason out and without the pending debits (`param_6`, `param_7`
/// 0): the player can pay `count` units of its master `master`. A master the player lacks or
/// of a type below 70 cannot. Each cost entry of the costed master ([`costed`]) with an
/// attribute (and paid, for an annex's) needs its amount times `count` at most what the
/// attribute holds (`COMISS`, `JA`: a NaN passes); the population's entry (attribute 4) is
/// tested only with `population` (`param_5` 0): the population (attribute 11) below the cap
/// (32), and the queued units (80) plus the population plus the need at most the cap or the
/// headroom (4) plus the population, the smaller.
pub fn cost_test(
    masters: &Masters,
    master: i16,
    count: f32,
    population: bool,
    attributes: &dyn crate::technology::Attributes,
) -> Result<bool, GenieError> {
    let Some(m) = masters.get(master).filter(|m| COMBAT_MASTER <= m.kind) else {
        return Ok(false);
    };
    let (m, annex) = costed(masters, m);
    for j in 0..3 {
        let (attribute, amount, paid) = cost_entry(m, j);
        if attribute < 0 || (annex && !paid) {
            continue;
        }
        let need = f32::from(amount) * count;
        if attribute == HEADROOM {
            if !population {
                continue;
            }
            let pop = attributes.get(POPULATION as i16)?;
            let cap = attributes.get(POPULATION_CAP as i16)?;
            if cap <= pop {
                return Ok(false);
            }
            let room = attributes.get(HEADROOM)? + pop;
            // CMOVBE: the cap unless it is above the headroom's sum (a NaN takes the cap)
            let limit = if crate::math::fmath::at_most_or_unordered(cap, room) {
                cap
            } else {
                room
            };
            if limit < attributes.get(QUEUED)? + pop + need {
                return Ok(false);
            }
        } else if attributes.get(attribute)? < need {
            return Ok(false);
        }
    }
    Ok(true)
}

/// `FUN_14162fb60`: when the player can pay `count` units of master `master` without the
/// population's test ([`cost_test`] with `param_5` 1), each of the master's own cost entries
/// with an attribute and a paid flag comes off its attribute, its amount times `count`
/// (straight into the attribute; the statistics `FUN_14097daf0` are not the simulation's).
pub fn pay(
    masters: &Masters,
    master: i16,
    count: f32,
    attributes: &mut dyn crate::technology::Attributes,
) -> Result<bool, GenieError> {
    if !cost_test(masters, master, count, false, &*attributes)? {
        return Ok(false);
    }
    // the cost test found the master
    let Some(m) = masters.get(master) else {
        return Ok(false);
    };
    for j in 0..3 {
        let (attribute, amount, paid) = cost_entry(m, j);
        if attribute < 0 || !paid {
            continue;
        }
        attributes.subtract(attribute, f32::from(amount) * count)?;
    }
    Ok(true)
}

/// `FUN_14162ef60` (no type out, the multiplier 1.0): master `master`'s three costs as a queue
/// entry records them. Each entry with an attribute takes its amount (times 1.0, truncated to
/// 16 bits); when the player could pay one more ([`cost_test`] with the population) yet holds
/// less of an attribute than its amount, the entry takes what it holds (truncated) and the
/// difference goes to gold: to the first entry of no attribute or of gold (attribute 3, its
/// amount grown; the third takes the difference alone). An amount below 1 leaves no attribute.
/// A master the player lacks or of a type below 70 lists none.
pub fn master_costs(
    masters: &Masters,
    master: i16,
    attributes: &dyn crate::technology::Attributes,
) -> Result<[(i16, i16); 3], GenieError> {
    use crate::checkpoint::cvttss2si;
    let mut out = [(-1i16, 0i16); 3];
    let can = cost_test(masters, master, ONE, true, attributes)?;
    let Some(m) = masters.get(master).filter(|m| COMBAT_MASTER <= m.kind) else {
        return Ok(out);
    };
    let mut extra = 0i16;
    for (j, slot) in out.iter_mut().enumerate() {
        let (attribute, amount, _) = cost_entry(m, j);
        slot.0 = attribute;
        if attribute < 0 {
            continue;
        }
        slot.1 = cvttss2si(f32::from(amount) * ONE) as i16;
        let held = attributes.get(attribute)?;
        if held < f32::from(slot.1) && can {
            let have = cvttss2si(held) as i16;
            extra = extra.wrapping_add(slot.1.wrapping_sub(have));
            slot.1 = have;
        }
    }
    if 0 < extra {
        let gold = |a: i16| matches!(a, -1 | 3);
        if gold(out[0].0) {
            out[0] = (3, out[0].1.wrapping_add(extra));
        } else if gold(out[1].0) {
            out[1] = (3, out[1].1.wrapping_add(extra));
        } else if gold(out[2].0) {
            out[2] = (3, extra);
        }
    }
    for slot in &mut out {
        if slot.1 < 1 {
            slot.0 = -1;
        }
    }
    Ok(out)
}

/// The costs a queue entry records from listed ones (`FUN_1411c3190`, `FUN_1411c81a0`): each
/// pair's attribute and its amount as a float, added to the slot's cleared 0.
#[must_use]
pub fn as_entry_costs(listed: [(i16, i16); 3]) -> [(i32, f32); 3] {
    listed.map(|(a, n)| (i32::from(a), f32::from(n) + 0.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(master: i16, count: i16, food: f32) -> Entry {
        Entry {
            master,
            count,
            tech: -1,
            costs: [(0, food), (-1, 0.0), (-1, 0.0)],
            serial: 0,
            button: -1,
            train_time: 25,
        }
    }

    fn queue(entries: &[Entry]) -> Queue {
        let mut slots = vec![Entry::default(); 10];
        slots[..entries.len()].copy_from_slice(entries);
        for (k, s) in slots.iter_mut().enumerate() {
            s.serial = k as i64;
        }
        Queue {
            slots,
            count: entries.len() as i16,
            total: entries.iter().map(|e| e.count).sum(),
            next_serial: 10,
            ..Queue::default()
        }
    }

    /// `FUN_1411d66b0` walks the units entry by entry: index 4 of [3 archers, 2 villagers] is
    /// the second villager, index 5 none.
    #[test]
    fn a_unit_index_walks_the_entries_counts() {
        let q = queue(&[unit(4, 3, 25.0), unit(83, 2, 50.0)]);
        let at: Vec<_> = (0..6).map(|i| q.entry_of_unit(i)).collect();
        assert_eq!(at, [Some(0), Some(0), Some(0), Some(1), Some(1), None]);
        assert_eq!(Queue::default().entry_of_unit(0), None);
    }

    /// An emptied entry goes: the later ones move down field by field, the serials stay, the
    /// old last slot keeps its copy; without `time` the train times stay as they were.
    #[test]
    fn closing_an_entry_moves_the_later_ones_down_but_not_their_serials() {
        let mut q = queue(&[unit(4, 0, 25.0), unit(83, 2, 50.0), unit(5, 1, 30.0)]);
        q.slots[2].train_time = 40;
        q.close(0, true);
        assert_eq!(q.count, 2);
        assert_eq!(
            q.slots[..3]
                .iter()
                .map(|s| (s.master, s.count, s.serial, s.train_time))
                .collect::<Vec<_>>(),
            [(83, 2, 0, 25), (5, 1, 1, 40), (5, 1, 2, 40)]
        );
        // a research's cancel (FUN_1411d61e0) leaves the train times in their slots
        let mut q = queue(&[unit(4, 0, 25.0), unit(83, 2, 50.0)]);
        q.slots[0].train_time = 7;
        q.close(0, false);
        assert_eq!((q.slots[0].master, q.slots[0].train_time), (83, 7));
    }

    /// The regrowth by ten keeps every old slot but its costs, which it fills anew.
    #[test]
    fn a_full_buffer_grows_by_ten_and_fills_the_old_costs_anew() {
        let mut q = queue(&[unit(83, 1, 50.0)]);
        q.slots[3] = unit(7, 0, 9.0);
        assert!(!q.full());
        let refill: Vec<_> = q
            .slots
            .iter()
            .map(|s| {
                [
                    (i32::from(s.master), f32::from(s.tech)),
                    NO_COSTS[1],
                    NO_COSTS[2],
                ]
            })
            .collect();
        q.grow(&refill);
        assert_eq!(q.slots.len(), 20);
        assert_eq!(q.slots[0].costs[0], (83, -1.0));
        assert_eq!((q.slots[3].master, q.slots[3].costs[0]), (7, (7, -1.0)));
        assert_eq!(q.slots[15], Entry::default());
        // a new entry takes the next serial and no costs
        let i = q.open(5, 2, -1, 3);
        assert_eq!((i, q.slots[1].serial, q.slots[1].costs), (1, 10, NO_COSTS));
        assert_eq!(q.next_serial, 11);
    }

    #[test]
    fn the_first_entry_lets_one_unit_go_and_then_itself() {
        let mut q = queue(&[unit(83, 2, 50.0), unit(4, 1, 25.0)]);
        q.release_first();
        assert_eq!(
            (q.count, q.total, q.changes, q.slots[0].count),
            (2, 2, 1, 1)
        );
        q.release_first();
        assert_eq!(
            (q.count, q.total, q.changes, q.slots[0].master),
            (1, 1, 2, 4)
        );
    }

    /// `FUN_1411cbc40`'s join compares the amounts as floats: a NaN never joins.
    #[test]
    fn costs_join_only_when_equal_and_not_nan() {
        let a = [(0, 50.0), (-1, 0.0), (-1, 0.0)];
        assert!(same_costs(&a, &a));
        assert!(!same_costs(&a, &[(1, 50.0), (-1, 0.0), (-1, 0.0)]));
        let nan = [(0, f32::NAN), (-1, 0.0), (-1, 0.0)];
        assert!(!same_costs(&nan, &nan));
    }

    /// A villager at the town centre (109) with button 1 trains in 25 seconds there; another
    /// building or button takes the first location's time; no locations, 0.
    #[test]
    fn a_train_time_is_the_building_s_and_button_s_else_the_first() {
        let at = |location, time, button| TrainLocation {
            location,
            time,
            button,
            hotkey: 0,
        };
        let locations = [at(82, 30, 2), at(109, 25, 1)];
        assert_eq!(train_time(&locations, 109, 1), 25);
        assert_eq!(train_time(&locations, 109, 2), 30);
        assert_eq!(train_time(&locations, 109, -1), 30);
        assert_eq!(train_time(&locations, 109, 0x101), 25);
        assert_eq!(train_time(&[], 109, 1), 0);
        let mut m = Master {
            kind: 70,
            train_locations: locations.to_vec(),
            ..Master::default()
        };
        let mut make = MakeObj::for_entry(&Entry {
            master: 83,
            button: 1,
            train_time: 99,
            ..Entry::default()
        });
        assert_eq!(make.time(Some(&m), 109), 25);
        m.object_flags = 0x80;
        assert_eq!(make.time(Some(&m), 109), 99);
        m.kind = 60;
        assert_eq!(make.time(Some(&m), 109), 0);
        make.button = -1;
        assert_eq!(
            (make.time(None, 109), make.paid, make.unique_id),
            (0, true, -1)
        );
    }

    /// A villager stores −1 headroom, +1 population and +1 of attribute 19.
    #[test]
    fn a_villager_takes_one_population() {
        assert_eq!(population(&[(4, -1.0, 2), (11, 1.0, 2), (19, 1.0, 1)]), 1.0);
        let mut a = vec![0.0; 82];
        (a[4], a[11], a[32], a[81]) = (6.0, 4.0, 5.0, 0.0);
        assert!(!housed(&a, 1.0).unwrap());
        a[81] = 1.0;
        assert!(housed(&a, 1.0).unwrap());
        a[32] = f32::NAN;
        a[4] = 10.0;
        assert!(!housed(&a, 1.0).unwrap());
    }

    struct Held(Vec<f32>);

    impl crate::technology::Attributes for Held {
        fn get(&self, i: i16) -> Result<f32, GenieError> {
            Ok(self.0[i as usize])
        }
        fn set(&mut self, i: i16, v: f32) -> Result<(), GenieError> {
            self.0[i as usize] = v;
            Ok(())
        }
    }

    /// `FUN_1413ccc00` with `param_3` 1: a technology in state 2 or 4 gives back each paid cost
    /// entry's amount plus its adjustment, scaled by the research cost modifier (attribute 85)
    /// above 0 and truncated to 16 bits (not an age's), and is in state 1 with no work done; in
    /// another state nothing happens.
    #[test]
    fn a_cancelled_research_gives_back_its_paid_costs() {
        use crate::technology::{Cost, TechDef, TechState};
        let def = TechDef {
            costs: [
                Cost {
                    attribute: 0,
                    amount: 100,
                    paid: 1,
                },
                Cost {
                    attribute: 1,
                    amount: 50,
                    paid: 0,
                },
                Cost {
                    attribute: -1,
                    amount: 5,
                    paid: 1,
                },
            ],
            ..TechDef::default()
        };
        let state = |state| TechState {
            state,
            research_done: 7.5,
            cost_adj: [-10, 0, 0],
            ..TechState::default()
        };
        let refunded = |state: &mut TechState, tech, modifier| {
            let mut held = Held(vec![0.0; 90]);
            held.0[usize::try_from(RESEARCH_COST_MODIFIER).unwrap()] = modifier;
            let r = refund_tech(state, &def, tech, &mut held);
            (r, held.0[0], held.0[1])
        };
        let mut s = state(2);
        assert_eq!(refunded(&mut s, 22, 0.0), (Ok(true), 90.0, 0.0));
        assert_eq!((s.state, s.research_done), (1, 0.0));
        let mut s = state(4);
        assert_eq!(refunded(&mut s, 22, 0.75), (Ok(true), 67.0, 0.0));
        let mut s = state(4);
        assert_eq!(refunded(&mut s, 101, 0.75), (Ok(true), 90.0, 0.0));
        // the scaled amount wraps at 16 bits: 40000 is -25536
        let mut s = TechState {
            cost_adj: [19_900, 0, 0],
            ..state(2)
        };
        assert_eq!(refunded(&mut s, 22, 2.0), (Ok(true), -25_536.0, 0.0));
        let mut s = state(3);
        assert_eq!(refunded(&mut s, 22, 0.0), (Ok(false), 0.0, 0.0));
        assert_eq!((s.state, s.research_done), (3, 7.5));
    }

    /// `FUN_141630530` gives back what the entry paid of each paid cost (the last pair naming
    /// it), not what the master costs now; an unpaid cost and a master below type 70 give
    /// nothing.
    #[test]
    fn a_refund_gives_back_what_the_entry_paid() {
        let m = Master {
            kind: 70,
            cost: vec![(0, 60, true), (3, 20, true), (4, 1, false)],
            ..Master::default()
        };
        let mut held = Held(vec![1.0; 5]);
        refund(&m, &[(0, 50.0), (0, 45.0), (4, 1.0)], &mut held).unwrap();
        assert_eq!(held.0, [46.0, 1.0, 1.0, 1.0, 1.0]);
        let low = Master {
            kind: 60,
            ..m.clone()
        };
        refund(&low, &[(0, 50.0), (-1, 0.0), (-1, 0.0)], &mut held).unwrap();
        assert_eq!(held.0[0], 46.0);
        refund_research(&[(3, 50.0), (-1, 0.0), (0, 0.5)], 3, &mut held).unwrap();
        assert_eq!((held.0[3], held.0[0]), (151.0, 47.5));
        refund_master(&m, 1, &mut held).unwrap();
        assert_eq!((held.0[0], held.0[3], held.0[4]), (107.5, 171.0, 1.0));
    }

    fn villager(cost: Vec<(i16, i16, bool)>) -> Master {
        Master {
            id: 83,
            kind: 70,
            cost,
            ..Master::default()
        }
    }

    /// 100 food, headroom 5, population 4, cap 10, nothing queued.
    fn held() -> Held {
        let mut a = vec![0.0; 82];
        (a[0], a[4], a[11], a[32]) = (100.0, 5.0, 4.0, 10.0);
        Held(a)
    }

    /// `FUN_141618db0`: two villagers' 100 food are held, three's are not; the population's
    /// entry asks the queued units plus the population plus the need to stay within the cap
    /// or the headroom's sum, the smaller, only with `param_5` 0.
    #[test]
    fn the_cost_test_reads_each_entry_and_the_population_only_when_asked() {
        let ms = Masters::new([villager(vec![(0, 50, true), (4, 1, false), (-1, 0, false)])]);
        let mut h = held();
        assert!(cost_test(&ms, 83, 2.0, true, &h).unwrap());
        assert!(!cost_test(&ms, 83, 3.0, true, &h).unwrap());
        // 4 queued: 4 + 4 + 1 is the headroom's 9; one more queued is not
        h.0[80] = 4.0;
        assert!(cost_test(&ms, 83, 1.0, true, &h).unwrap());
        h.0[80] = 5.0;
        assert!(!cost_test(&ms, 83, 1.0, true, &h).unwrap());
        assert!(cost_test(&ms, 83, 1.0, false, &h).unwrap());
        // a population at the cap
        let mut h = held();
        h.0[32] = 4.0;
        assert!(!cost_test(&ms, 83, 1.0, true, &h).unwrap());
        // a NaN need passes, a master the player lacks or below type 70 fails
        assert!(cost_test(&ms, 83, f32::NAN, false, &h).unwrap());
        assert!(!cost_test(&ms, 84, 1.0, false, &h).unwrap());
        let low = Masters::new([Master {
            kind: 60,
            ..villager(vec![])
        }]);
        assert!(!cost_test(&low, 83, 1.0, false, &h).unwrap());
    }

    /// `FUN_14162fb60` pays the paid entries times the count in one subtraction.
    #[test]
    fn a_payment_takes_the_paid_entries_times_the_count() {
        let ms = Masters::new([villager(vec![(0, 50, true), (4, 1, false), (1, 7, true)])]);
        let mut h = held();
        h.0[1] = 30.0;
        assert!(pay(&ms, 83, 2.0, &mut h).unwrap());
        assert_eq!((h.0[0], h.0[1], h.0[4]), (0.0, 16.0, 5.0));
        assert!(!pay(&ms, 83, 1.0, &mut h).unwrap());
        assert_eq!(h.0[0], 0.0);
    }

    /// An annex (master 2, head 3 whose stack unit is 4) is tested by its stack unit's paid
    /// costs, while `FUN_14162ef60` lists its own: what it lacks of food and stone goes to the
    /// third entry as gold; a gold entry first takes it on its amount; an amount below 1 lists
    /// no attribute.
    #[test]
    fn listed_costs_move_what_the_player_lacks_to_gold() {
        use crate::objects::masters::BuildingMaster;
        let building = |id, head, stack, cost| Master {
            id,
            kind: 80,
            cost,
            building: Some(BuildingMaster {
                head_unit: head,
                stack_unit: stack,
                ..BuildingMaster::default()
            }),
            ..Master::default()
        };
        let ms = Masters::new([
            building(
                2,
                3,
                -1,
                vec![(0, 50, false), (2, 30, false), (-1, 0, false)],
            ),
            building(3, -1, 4, vec![]),
            building(4, -1, -1, vec![(1, 10, true), (0, 500, false)]),
        ]);
        let mut h = held();
        (h.0[0], h.0[1], h.0[2]) = (20.0, 100.0, 5.5);
        assert_eq!(costed(&ms, ms.get(2).unwrap()).0.id, 4);
        assert_eq!(
            master_costs(&ms, 2, &h).unwrap(),
            [(0, 20), (2, 5), (3, 55)]
        );
        // a gold entry first takes the difference on what it holds
        let ms = Masters::new([
            building(
                2,
                3,
                -1,
                vec![(3, 50, false), (0, 30, false), (1, 0, false)],
            ),
            building(3, -1, 4, vec![]),
            building(4, -1, -1, vec![(1, 10, true)]),
        ]);
        (h.0[3], h.0[0]) = (20.0, 10.0);
        assert_eq!(
            master_costs(&ms, 2, &h).unwrap(),
            [(3, 70), (0, 10), (-1, 0)]
        );
        // a unit the player cannot pay for one more of moves nothing
        let ms = Masters::new([villager(vec![
            (3, 50, true),
            (-1, 0, false),
            (-1, 0, false),
        ])]);
        assert_eq!(
            master_costs(&ms, 83, &h).unwrap(),
            [(3, 50), (-1, 0), (-1, 0)]
        );
        assert_eq!(master_costs(&ms, 7, &h).unwrap(), [(-1, 0); 3]);
        assert_eq!(
            as_entry_costs([(3, 50), (-1, 0), (-1, -2)]),
            [(3, 50.0), (-1, 0.0), (-1, -2.0)]
        );
    }

    #[test]
    fn a_shared_research_joins_both_lists_once_and_a_cancel_drops_it() {
        let mut p = Production::default();
        p.note_shared_research(7);
        p.note_shared_research(7);
        assert_eq!(
            (p.researched_techs.as_slice(), p.disabled_techs.as_slice()),
            (&[7][..], &[7][..])
        );
        p.disabled_techs.extend([3, 7]);
        p.enable_tech(7);
        assert_eq!(p.disabled_techs, [3]);
        assert!(p.queue.enabled);
    }
}
