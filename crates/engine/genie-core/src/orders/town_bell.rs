//! A villager's town-bell record (`+0x9ac`..`+0x9dc`): what it worked on when an order to a
//! target took it off its work (`FUN_141206a20`), which a later back-to-work order sends it back
//! to, and the flag a garrison order sets (`FUN_1412067f0`). Saved as `mTownBellFlag`,
//! `mBackToWork1` and `mBackToWork2`.

use crate::GenieError;
use crate::gathering::gather::GATHER;
use crate::gathering::hunt::HUNT;
use crate::movement::footprint_gap_squared;
use crate::objects::ObjectId;
use crate::objects::masters::class;
use crate::objects::masters::ids::MASTER_1808;
use crate::objects::masters::ids::TOWN_CENTRE;
use crate::objects::masters::master_type::BUILDING;
use crate::objects::masters::master_type::COMBAT;
use crate::orders::{GARRISON, own_order_target};
use crate::players::PlayerId;
use crate::unit_ai::ActionKind;
use crate::unit_ai::task_finder::Diplomacy;
use crate::unit_ai::task_finder::world::TaskWorld;
use crate::world::World;
use genie_diff::Diff;

/// One back-to-work record.
#[derive(Clone, Copy, Debug, PartialEq, Diff)]
pub struct BackToWork {
    /// `townBellTargetId` (`+0x9ac`): the top action's target (the pasture of a target in one),
    /// else the target of the unit AI's first own queued order, −1 for none.
    pub target: i32,
    /// `town_bell_target_type` (`+0x9b0`): a Gather's (5) or Hunt's (`0x6e`) target class
    /// (`+0x68`), else −1.
    pub target_type: i32,
    /// `town_bell_action` (`+0x9b4`): the unit AI's current action (`+0x44`).
    pub action: Option<ActionKind>,
    /// `townBellTargetId2` (`+0x9b8`): the top action's second target (`+0x34`), −1 for none.
    pub target2: i32,
    /// `town_bell_target_x`, `_y` (`+0x9bc`, `+0x9c0`): the target's place, else the unit's own.
    pub point: [f32; 2],
}

impl BackToWork {
    /// A record of nothing, as a new unit's (`FUN_14121df60`) and the second record a work other
    /// than building drops: ids −1, the point (−1, −1).
    pub const NONE: Self = Self {
        target: -1,
        target_type: -1,
        action: None,
        target2: -1,
        point: [-1.0, -1.0],
    };
}

/// The town-bell flag and the two records of a combat object (type 70 and up).
#[derive(Clone, Copy, Debug, PartialEq, Diff)]
pub struct TownBell {
    /// `mTownBellFlag` (`+0x9dc`): a garrison order sent the villager into a building that takes
    /// villagers; no record is made while it holds.
    pub flag: bool,
    /// `mBackToWork1` (`+0x9ac`..`+0x9c0`): the work it was taken off last.
    pub work: BackToWork,
    /// `mBackToWork2` (`+0x9c4`..`+0x9d8`): the work before a building it was taken off.
    pub before_building: BackToWork,
}

impl Default for TownBell {
    fn default() -> Self {
        Self {
            flag: false,
            work: BackToWork::NONE,
            before_building: BackToWork::NONE,
        }
    }
}

impl TownBell {
    /// `FUN_141206a20`: with the flag clear, the unit's present work replaces the first record
    /// when any of its fields differs. The first record moves to the second when the new work is
    /// building (`0x25a`) and the old was not; any other new work clears the second (its ids −1,
    /// its point (−1, −1)). The new record ([`BackToWork`]) is the caller's: its target, target
    /// class, the unit AI's action, the second target and the point.
    pub fn record(&mut self, now: BackToWork) {
        if self.flag || now == self.work {
            return;
        }
        if now.action == Some(ActionKind::Build) {
            if self.work.action != Some(ActionKind::Build) {
                self.before_building = self.work;
            }
        } else {
            self.before_building = BackToWork::NONE;
        }
        self.work = now;
    }
}

/// The unit AI's actions whose record a finished building ends: building (`0x25a`), gathering
/// (`0x261`) and hunting (`0x265`) (`FUN_1411f8fa0`'s mask `0x881`).
const ENDED_ON_A_BUILDING: [ActionKind; 3] =
    [ActionKind::Build, ActionKind::Gather, ActionKind::Hunt];

/// What a back-to-work record's check (`FUN_1411f8fa0`) reads of an object the world holds:
/// whether it is a building (type 80), its master's class, and its build points (`+0xc50`)
/// against its master's build time (the first train location's train time).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecordTarget {
    /// The object.
    pub id: i32,
    /// Its master is of type 80.
    pub building: bool,
    /// Its master's class.
    pub class: i16,
    /// It has had its master's build time of work (`build_time <= +0xc50`).
    pub built: bool,
}

/// `FUN_1411f8fa0`: the object a record names, its target, else its second target, the world
/// holds (`lookup`); for the actions of [`ENDED_ON_A_BUILDING`] a building other than a farm
/// that is built ends the record (no object, both targets −1 in the copy returned).
fn record_object(
    record: BackToWork,
    lookup: &dyn Fn(i32) -> Option<RecordTarget>,
) -> (Option<RecordTarget>, BackToWork) {
    let mut out = record;
    let mut found = (0 <= record.target)
        .then(|| lookup(record.target))
        .flatten();
    if found.is_none() {
        if record.target2 < 0 {
            return (None, out);
        }
        found = lookup(record.target2);
    }
    if let Some(o) = found
        && record
            .action
            .is_some_and(|a| ENDED_ON_A_BUILDING.contains(&a))
        && o.building
        // The farms' class, whose records a built farm does not end.
        && o.class != class::FARM
        && o.built
    {
        out.target = -1;
        out.target2 = -1;
        return (None, out);
    }
    (found, out)
}

impl TownBell {
    /// `FUN_1412035a0`: the work to go back to and the object it is on. The first record's
    /// object ([`record_object`]) wins; else the second's, which then becomes the first (the
    /// second cleared); else no object and the first record as it is, or the second when the
    /// first has no point. The record returned keeps the point of the one whose object was
    /// found.
    pub fn back_to_work(
        &mut self,
        lookup: &dyn Fn(i32) -> Option<RecordTarget>,
    ) -> (Option<RecordTarget>, BackToWork) {
        let (found, out) = record_object(self.work, lookup);
        if found.is_some() {
            return (found, out);
        }
        let before = self.before_building;
        let (found, out) = record_object(before, lookup);
        if found.is_some() {
            self.work = before;
            self.before_building = BackToWork::NONE;
            return (found, out);
        }
        let none = [-1.0, -1.0];
        if self.work.point == none {
            (None, before)
        } else {
            (None, self.work)
        }
    }
}

/// The town bell's reach ([`crate::garrison::TOWN_BELL_REACH`]).
const REACH: f32 = crate::garrison::TOWN_BELL_REACH;
/// A record's point on neither axis (`DAT_14330ec20`).
const NO_POINT: f32 = crate::math::image::garrison::NO_POINT;

impl World {
    /// `TOWN_BELL` (0x7f, `do_command`): the object `id` (none for −1, `FUN_140e6bad0`) of the
    /// commanding `player`, or of any player while players may order each other's
    /// ([`crate::world::settings::Settings::cheats_enabled`], `FUN_1412ffe00`; the cheat `+0xb12` is not held),
    /// rings its owner's bell (`activate`, [`crate::world::World::ring_town_bell`]) or ends it
    /// ([`crate::world::World::end_town_bell`]). A special object's (`+0x30c` bit 0) UI flag is
    /// not simulation.
    pub(crate) fn town_bell(
        &mut self,
        player: PlayerId,
        id: ObjectId,
        activate: bool,
    ) -> Result<(), GenieError> {
        let player = player.index();
        let id = id.get();
        let Some((owner, _)) = (id != -1).then(|| self.object(id)).flatten() else {
            return Ok(());
        };
        if owner != player && !self.settings.cheats_enabled {
            return Ok(());
        }
        if activate {
            self.ring_town_bell(owner, id)
        } else {
            self.end_town_bell(owner, id)
        }
    }

    /// `FUN_14163a650`: player `owner`'s town centre `tc` (master 109; any other object does
    /// nothing) rings: of its owner's live objects, in the list's order, those in state 2 not
    /// inside another (`FUN_141188da0`) within [`REACH`] of it (`FUN_14118f8c0`; `COMISS`/`JC`:
    /// a NaN gap is out) are the units, a villager (class 4 of type 70) or master 1808, else
    /// the buildings (type 80) that take villagers (`garrison_type` bit 0, `FUN_1411ccb60`), each
    /// with its count ([`crate::world::World::garrisoned`]) and its bell flag (`+0x9dc`) set (its
    /// sound, slot 0x4c0, is not simulation); then [`crate::world::World::shelter`] with the
    /// reach (`param_6` 1), no transports, not through the unit AIs.
    pub(crate) fn ring_town_bell(&mut self, owner: usize, tc: i32) -> Result<(), GenieError> {
        let Some((_, t)) = self.object(tc) else {
            return Ok(());
        };
        // The town centre, whose bell the command rings (`0x6109` raw).
        // The master that answers the bell besides the villagers (`0x710`).
        // The villagers' class.
        // The units' least type (`0x46`) and the buildings' (`'P'`).
        if t.life.master_id != TOWN_CENTRE {
            return Ok(());
        }
        let (at, radius) = self.ground(tc)?;
        let mut units = Vec::new();
        let mut shelters = crate::garrison::Shelters::default();
        let mut k = 0;
        while k < self.players[owner].objects.len() {
            let o = &self.players[owner].objects[k];
            k += 1;
            if o.life.state != 2 || o.life.inside.is_some() {
                continue;
            }
            let id = o.id;
            let m = o.master(&self.masters, owner)?;
            let (kind, class, master) = (m.kind, m.class, m.id);
            let takes_villagers = m.building.is_some_and(|b| b.garrison_type & 1 != 0);
            let near = |t: &Self| -> Result<bool, GenieError> {
                let (o_at, o_radius) = t.ground(id)?;
                let gap = footprint_gap_squared(o_at, o_radius, at, radius);
                // COMISS REACH, gap; JC: a gap above it, or a NaN, is out
                Ok(!crate::math::fmath::below_or_unordered(REACH, gap))
            };
            if (class == class::VILLAGER && kind == COMBAT && near(self)?)
                || (master == MASTER_1808 && near(self)?)
            {
                units.push(id);
            } else if kind == BUILDING && takes_villagers && near(self)? {
                shelters.buildings.push(id);
                shelters.counts.push(self.garrisoned(id)?);
                self.players[owner].objects[k - 1].town_bell.flag = true;
            }
        }
        self.shelter(owner, &units, &shelters, true, false)
    }

    /// `FUN_14163ab80`: player `owner`'s town centre `tc` (master 109; any other object does
    /// nothing) ends its bell. Each of its owner's live objects, in the list's order, of type 80
    /// in a state below 3 that takes villagers (`FUN_1411ccb60` with 1) and holds its bell flag
    /// (`+0x9dc`) drops the flag; within [`REACH`] of the centre (`COMISS`/`JC`: a NaN gap is
    /// out) it lets each unit inside of type 70 or more that is a villager (class 4) or of master
    /// 1808 back to work, from the last in ([`crate::world::World::back_to_work`]; its sound, slot
    /// 0x4c0, is not simulation). Then every unit its owner's tracker files under class 4
    /// (`FUN_14104f8f0`) and master 1808 (`FUN_14104f960`), in that order, not inside another
    /// (`+0x2f0`) goes back to work from where it is.
    pub(crate) fn end_town_bell(&mut self, owner: usize, tc: i32) -> Result<(), GenieError> {
        let Some((_, t)) = self.object(tc) else {
            return Ok(());
        };
        if t.life.master_id != TOWN_CENTRE {
            return Ok(());
        }
        let (at, radius) = self.ground(tc)?;
        let mut k = 0;
        while k < self.players[owner].objects.len() {
            let idx = k;
            k += 1;
            let b = &self.players[owner].objects[idx];
            let m = b.master(&self.masters, owner)?;
            let takes_villagers = m.building.is_some_and(|x| x.garrison_type & 1 != 0);
            if m.kind != BUILDING || 3 <= b.life.state || !takes_villagers || !b.town_bell.flag {
                continue;
            }
            let id = b.id;
            self.players[owner].objects[idx].town_bell.flag = false;
            let (b_at, b_radius) = self.ground(id)?;
            // COMISS REACH, gap; JC: a gap above it, or a NaN, lets none out
            let gap = footprint_gap_squared(b_at, b_radius, at, radius);
            if crate::math::fmath::below_or_unordered(REACH, gap) {
                continue;
            }
            self.workers_out(owner, idx)?;
        }
        self.workers_back(owner)
    }

    /// `ALL_BACK_TO_WORK` (`do_command` 0x8c; another player's object only while players may
    /// order each other's, [`crate::world::settings::Settings::cheats_enabled`]; the cheat `+0xb12` is
    /// not held): `FUN_141617420` for the owner of object `id`, as the bell's end
    /// ([`crate::world::World::end_town_bell`]) without its flags: each of the owner's live
    /// objects, in the list's order, in state 2, not inside another (`+0x2f0`), of type 80, that
    /// takes villagers (`FUN_1411ccb60` with 1) and is within [`REACH`] of the object
    /// (`COMISS`/`JC`: a NaN gap is out) lets its villagers and units of master 1808 out back to
    /// work, from the last in; then the units its owner's tracker files under class 4 and master
    /// 1808 not inside another go back from where they are. The game screen's flag (`+0x1874`) is
    /// not simulation.
    pub(crate) fn all_back_to_work(
        &mut self,
        player: PlayerId,
        id: ObjectId,
    ) -> Result<(), GenieError> {
        let player = player.index();
        let id = id.get();
        let Some((owner, _)) = self.object(id) else {
            return Ok(());
        };
        if owner != player && !self.settings.cheats_enabled {
            return Ok(());
        }
        let (at, radius) = self.ground(id)?;
        let mut k = 0;
        while k < self.players[owner].objects.len() {
            let idx = k;
            k += 1;
            let b = &self.players[owner].objects[idx];
            let m = b.master(&self.masters, owner)?;
            let takes_villagers = m.building.is_some_and(|x| x.garrison_type & 1 != 0);
            if b.life.state != 2
                || b.life.inside.is_some()
                || m.kind != BUILDING
                || !takes_villagers
            {
                continue;
            }
            let (b_at, b_radius) = self.ground(b.id)?;
            // COMISS REACH, gap; JC: a gap above it, or a NaN, lets none out
            let gap = footprint_gap_squared(b_at, b_radius, at, radius);
            if crate::math::fmath::below_or_unordered(REACH, gap) {
                continue;
            }
            self.workers_out(owner, idx)?;
        }
        self.workers_back(owner)
    }

    /// The building at `idx` of player `owner` lets each unit inside of type 70 or more that is a
    /// villager (class 4) or of master 1808 back to work, from the last in
    /// ([`crate::world::World::back_to_work`]; its sound, slot 0x4c0, is not simulation).
    fn workers_out(&mut self, owner: usize, idx: usize) -> Result<(), GenieError> {
        let mut i = self.players[owner].objects[idx].garrison.len();
        while 0 < i {
            i -= 1;
            let Some(&inside) = self.players[owner].objects[idx].garrison.get(i) else {
                continue;
            };
            // any player's unit; an id the world no longer holds is a null entry
            let Some((up, u)) = self.object(inside) else {
                continue;
            };
            let um = u.master(&self.masters, up)?;
            if COMBAT <= um.kind && (um.class == class::VILLAGER || um.id == MASTER_1808) {
                let _ = self.back_to_work(Some((owner, idx)), inside)?;
            }
        }
        Ok(())
    }

    /// Every unit player `owner`'s tracker files under class 4 (`FUN_14104f8f0`) and master
    /// 1808 (`FUN_14104f960`), in that order, not inside another (`+0x2f0`) goes back to work
    /// from where it is.
    fn workers_back(&mut self, owner: usize) -> Result<(), GenieError> {
        let tracker = &self.players[owner].tracker;
        let mut ids = tracker.class_list(i32::from(class::VILLAGER)).to_vec();
        ids.extend_from_slice(tracker.list(i32::from(MASTER_1808)));
        for id in ids {
            // the tracker's keepers follow every object its player gains and loses: the world
            // holds each (the game reads it without a test)
            let Some((_, u)) = self.object(id) else {
                continue;
            };
            if u.life.inside.is_none() {
                let _ = self.back_to_work(None, id)?;
            }
        }
        Ok(())
    }

    /// `FUN_1412081e0`'s part for a unit outside a building (its `param_2` null), after the
    /// records' lookup: with its bell flag (`+0x9dc`) and the record's point not −1 on either
    /// axis (`UCOMISS`/`JP`/`JZ`: only an exact −1 stops it, a NaN goes on), it goes there: with
    /// no object to work on, slot 0x340 (`FUN_141216c50`, [`crate::world::World::move_aside`]) to
    /// the point on the ground, range 0, forced, not walking; else slot 0x7e8 on the object at
    /// the point with the carried-resource test, any task, `+0xa14` 0. Then it is no longer under
    /// the town bell (`+0xa14` takes 1, not held). True when it went.
    pub(crate) fn back_outside(
        &mut self,
        id: i32,
        work_on: Option<i32>,
        point: [f32; 2],
    ) -> Result<bool, GenieError> {
        let Some(crate::orders::Place::Live(up, ui)) = self.place(id) else {
            return Ok(false);
        };
        if !self.players[up].objects[ui].town_bell.flag {
            return Ok(false);
        }
        let [x, y] = point;
        if x == NO_POINT || y == NO_POINT {
            return Ok(false);
        }
        match work_on {
            None => self.move_aside(
                up,
                ui,
                crate::unit_ai::AiMove {
                    point: [x, y, 0.0],
                    range: 0.0,
                    force: true,
                    walk: false,
                },
            )?,
            Some(target) => self.order_work(
                up,
                ui,
                &crate::orders::Work {
                    target,
                    point: [x, y, 0.0],
                    carry_test: true,
                    filter: -1,
                    notified: false,
                },
            )?,
        }
        if let Some((_, u)) = self.object_mut(id) {
            u.town_bell.flag = false;
        }
        Ok(true)
    }

    /// `FUN_1412067f0` before slot 0x7e8's work command, for a villager or master 1808 and a
    /// target the world holds: the town-bell record ([`World::record_town_bell`]); then, with the
    /// garrison filter (3), the flag goes on for a building in state 2 of the player or an ally
    /// whose master takes villagers (`garrison_type` bit 0), with any other filter it goes off.
    /// The UI's flag for the viewing player's building (`+0x1874`) is not held.
    pub(crate) fn town_bell_order(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
        filter: i16,
    ) -> Result<(), GenieError> {
        let unit = &self.players[player].objects[idx];
        let master = unit.master(&self.masters, player)?;
        if !(master.class == class::VILLAGER || master.id == MASTER_1808) {
            return Ok(());
        }
        let Some((tp, t)) = self.object(target) else {
            return Ok(());
        };
        let takes = if filter == GARRISON {
            let tm = t.master(&self.masters, tp)?;
            let world = TaskWorld {
                players: &self.players,
                drop_sites: &self.drop_sites,
            };
            (tm.kind == BUILDING
                && t.life.state == 2
                && world.allied(player, tp, true)
                && tm.building.is_some_and(|b| b.garrison_type & 1 != 0))
            .then_some(true)
        } else {
            Some(false)
        };
        self.record_town_bell(player, idx)?;
        if let Some(flag) = takes {
            self.players[player].objects[idx].town_bell.flag = flag;
        }
        Ok(())
    }

    /// `FUN_141206a20`: the unit's present work as its town-bell record
    /// ([`crate::orders::town_bell::TownBell::record`]): its top action's target (slot 400), or, for one
    /// in a pasture the world holds, the pasture; without one the target of its unit AI's first
    /// own queued order (`FUN_140c4da50`: issued by the unit, with a target), else none at its own
    /// place. Then a Gather's or Hunt's target class (`+0x68`), its unit AI's current action
    /// (`+0x44`) and the top action's second target (`FUN_140a78430`). The targets count only
    /// where the world holds them.
    pub(crate) fn record_town_bell(&mut self, player: usize, idx: usize) -> Result<(), GenieError> {
        use crate::orders::town_bell::BackToWork;
        let unit = &self.players[player].objects[idx];
        if unit.town_bell.flag {
            return Ok(());
        }
        if unit.unmodelled_action.is_some() {
            return Err(GenieError::unsupported(
                "a town-bell record of an action not held",
            ));
        }
        let held = |id: i32| (0 <= id).then(|| self.object(id)).flatten();
        let mut found = unit.top_target().and_then(held).map(|(_, t)| t);
        if let Some(t) = found
            && t.pasture != -1
        {
            found = held(t.pasture).map(|(_, p)| p);
        }
        if found.is_none() {
            let own = unit
                .ai
                .as_ref()
                .map_or(-1, |ai| own_order_target(ai, unit.id));
            found = held(own).map(|(_, t)| t);
        }
        let (target, point) = found.map_or_else(
            || {
                let [x, y, _] = unit.position();
                (-1, [x, y])
            },
            |t| {
                let [x, y, _] = t.position();
                (t.id, [x, y])
            },
        );
        let actions = &unit.actions;
        let target_type = match unit.top_action_kind() {
            Some(HUNT) => actions.hunt().map_or(-1, |h| i32::from(h.target_class)),
            Some(GATHER) => actions.gather().map_or(-1, |g| g.target_type),
            _ => -1,
        };
        let ai = unit.ai.as_ref().ok_or(GenieError::unsupported(
            "a town-bell record of a unit without a unit AI",
        ))?;
        let target2 = unit
            .top_second_target()
            .and_then(held)
            .map_or(-1, |(_, t)| t.id);
        let now = BackToWork {
            target,
            target_type,
            action: ai.current_action,
            target2,
            point,
        };
        self.players[player].objects[idx].town_bell.record(now);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn work(target: i32, action: Option<ActionKind>, point: [f32; 2]) -> BackToWork {
        BackToWork {
            target,
            action,
            point,
            ..BackToWork::NONE
        }
    }

    #[test]
    fn a_new_work_replaces_the_record_and_building_keeps_the_one_before() {
        // MarvinBlue/426048731's 9181: its first record (no target, its own place), then the
        // INTERACT of update 748 while it builds 10028, then the one of 797 while it cuts a tree
        let mut bell = TownBell {
            work: work(-1, None, [29.0, 99.0]),
            ..TownBell::default()
        };
        let building = work(10028, Some(ActionKind::Build), [30.0, 88.0]);
        bell.record(building);
        assert_eq!(bell.work, building);
        assert_eq!(bell.before_building, work(-1, None, [29.0, 99.0]));
        let cutting = BackToWork {
            target_type: 15,
            ..work(9365, Some(ActionKind::Hunt), [24.5, 97.5])
        };
        bell.record(cutting);
        assert_eq!(bell.work, cutting);
        assert_eq!(bell.before_building, BackToWork::NONE);
        // the same work again changes nothing, nor does any work while the flag holds
        bell.before_building = building;
        bell.record(cutting);
        assert_eq!(bell.before_building, building);
        bell.flag = true;
        bell.record(work(1, Some(ActionKind::Wait), [3.0, 4.0]));
        assert_eq!(bell.work, cutting);
    }

    /// The objects of a test world: a house 7 that is built, a house 8 that is not, a farm 9
    /// that is built, a sheep 10.
    fn object(id: i32) -> Option<RecordTarget> {
        let (building, class, built) = match id {
            7 => (true, 3, true),
            8 => (true, 3, false),
            9 => (true, class::FARM, true),
            10 => (false, 58, false),
            _ => return None,
        };
        Some(RecordTarget {
            id,
            building,
            class,
            built,
        })
    }

    #[test]
    fn a_record_names_its_target_else_its_second_target_unless_a_built_building_ended_it() {
        // FUN_1411f8fa0: the target the world holds
        let shepherd = BackToWork {
            target2: 7,
            ..work(10, Some(ActionKind::Gather), [5.0, 6.0])
        };
        assert_eq!(record_object(shepherd, &object), (object(10), shepherd));
        // a target the world no longer holds: the second target, unless it is below 0
        let gone = BackToWork {
            target2: 10,
            ..work(11, Some(ActionKind::Hunt), [5.0, 6.0])
        };
        assert_eq!(record_object(gone, &object), (object(10), gone));
        let none = BackToWork {
            target2: -1,
            ..gone
        };
        assert_eq!(record_object(none, &object), (None, none));
        let lost = BackToWork {
            target2: 12,
            ..gone
        };
        assert_eq!(record_object(lost, &object), (None, lost));
        // a target below 0 is not looked up: the second target is
        let second = BackToWork {
            target2: 10,
            ..work(-1, Some(ActionKind::Build), [5.0, 6.0])
        };
        assert_eq!(record_object(second, &object), (object(10), second));
        // building, gathering and hunting (mask 0x881 over 0x25a..0x265) on a built building
        // other than a farm end the record: no object, both targets −1 in the copy
        for action in [ActionKind::Build, ActionKind::Gather, ActionKind::Hunt].map(Some) {
            let on_house = BackToWork {
                target2: 10,
                ..work(7, action, [5.0, 6.0])
            };
            let ended = BackToWork {
                target: -1,
                target2: -1,
                ..on_house
            };
            assert_eq!(record_object(on_house, &object), (None, ended));
            // reached through the second target, too
            let through_second = BackToWork {
                target2: 7,
                ..work(11, action, [5.0, 6.0])
            };
            let ended = BackToWork {
                target: -1,
                target2: -1,
                ..through_second
            };
            assert_eq!(record_object(through_second, &object), (None, ended));
            // a house not built yet and a built farm stay
            for kept in [8, 9] {
                let r = work(kept, action, [5.0, 6.0]);
                assert_eq!(record_object(r, &object), (object(kept), r));
            }
        }
        // the actions between them, and any other, keep a built building
        for action in [
            Some(ActionKind::Heal),
            Some(ActionKind::Move),
            Some(ActionKind::Follow),
            Some(ActionKind::Transport),
            Some(ActionKind::Defend),
            None,
        ] {
            let r = work(7, action, [5.0, 6.0]);
            assert_eq!(record_object(r, &object), (object(7), r));
        }
    }

    mod rings {
        use crate::garrison::tests::{
            ARCHER, VILLAGER, inside_tc, let_out_world, sized_world, tracked, walker,
        };
        use crate::test_ids::object_id;
        use crate::test_ids::player_id;
        use crate::world::World;

        fn enter(t: &World, id: i32) -> Option<i32> {
            t.object(id).unwrap().1.actions.enter().map(|e| e.target)
        }

        #[test]
        fn the_bell_sends_the_villagers_in_reach_to_the_buildings_in_reach_and_flags_them() {
            // FUN_14163a650: villager 1000 is within 25 tiles of town centre 100, 1001 is not,
            // archer 1002 is no villager; the centre takes villagers and is in reach of itself
            let mut t = sized_world(60, 60);
            walker(&mut t, 1, 1000, VILLAGER, [5.5, 5.5]);
            walker(&mut t, 1, 1001, VILLAGER, [45.5, 45.5]);
            walker(&mut t, 1, 1002, ARCHER, [6.5, 5.5]);
            t.town_bell(player_id(1), object_id(100), true).unwrap();
            assert_eq!(
                [1000, 1001, 1002].map(|id| enter(&t, id)),
                [Some(100), None, None]
            );
            assert!(t.object(100).unwrap().1.town_bell.flag);
        }

        #[test]
        fn the_bell_is_its_owners_or_any_players_with_allied_control() {
            let mut t = let_out_world();
            walker(&mut t, 1, 1000, VILLAGER, [5.5, 5.5]);
            // player 2 rings player 1's centre: refused
            t.town_bell(player_id(2), object_id(100), true).unwrap();
            assert_eq!(enter(&t, 1000), None);
            t.settings.cheats_enabled = true;
            t.town_bell(player_id(2), object_id(100), true).unwrap();
            assert_eq!(enter(&t, 1000), Some(100));
            // an object that is no town centre rings nothing; nor does an id the world lacks
            let mut t = let_out_world();
            walker(&mut t, 1, 1000, VILLAGER, [5.5, 5.5]);
            t.town_bell(player_id(1), object_id(1000), true).unwrap();
            t.town_bell(player_id(1), object_id(4242), true).unwrap();
            assert_eq!(enter(&t, 1000), None);
        }

        #[test]
        fn the_bells_end_lets_the_villagers_out_and_sends_the_others_back_by_the_tracker() {
            // FUN_14163ab80: the centre drops its flag and lets villager 1000 out toward its
            // record; villager 1001 outside, filed under class 4, walks back to its record's
            // point (slot 0x340); 1002 has no flag and stays; 1003 is not filed and stays
            let mut t = let_out_world();
            t.object_mut(100).unwrap().1.town_bell.flag = true;
            inside_tc(&mut t, 1, 1000, VILLAGER);
            walker(&mut t, 1, 1001, VILLAGER, [5.5, 5.5]);
            walker(&mut t, 1, 1002, VILLAGER, [6.5, 5.5]);
            walker(&mut t, 1, 1003, VILLAGER, [7.5, 5.5]);
            for (id, point) in [(1000, [4.5, 4.5]), (1001, [3.5, 3.5]), (1002, [2.5, 2.5])] {
                let v = &mut t.object_mut(id).unwrap().1.town_bell;
                v.flag = id != 1002;
                v.work = crate::orders::town_bell::BackToWork {
                    point,
                    ..crate::orders::town_bell::BackToWork::NONE
                };
            }
            t.object_mut(1003).unwrap().1.town_bell.flag = true;
            t.object_mut(1003).unwrap().1.town_bell.work.point = [1.5, 1.5];
            tracked(&mut t, VILLAGER, &[1000, 1001, 1002]);
            t.town_bell(player_id(1), object_id(100), false).unwrap();
            assert!(!t.object(100).unwrap().1.town_bell.flag);
            let v = t.object(1000).unwrap().1;
            assert_eq!((v.life.inside, v.town_bell.flag), (None, false));
            let goal = |t: &World, id: i32| {
                let u = t.object(id).unwrap().1;
                (
                    u.actions.move_to().copied().and_then(|m| m.goal),
                    u.town_bell.flag,
                )
            };
            assert_eq!(goal(&t, 1001), (Some([3.5, 3.5, 0.0]), false));
            assert_eq!(goal(&t, 1002), (None, false));
            assert_eq!(goal(&t, 1003), (None, true));
            // a record without a point on an axis (an exact -1) keeps the unit and its flag
            let mut t = let_out_world();
            walker(&mut t, 1, 1001, VILLAGER, [5.5, 5.5]);
            let v = &mut t.object_mut(1001).unwrap().1.town_bell;
            v.flag = true;
            v.work.point = [3.5, -1.0];
            tracked(&mut t, VILLAGER, &[1001]);
            t.town_bell(player_id(1), object_id(100), false).unwrap();
            assert_eq!(goal(&t, 1001), (None, true));
        }
        #[test]
        fn all_back_to_work_lets_the_villagers_out_and_sends_the_others_back_leaving_the_flag() {
            // ALL_BACK_TO_WORK (do_command 0x8c, FUN_141617420): as the bell's end, but with no
            // bell flag to test or drop: villager 1000 comes out of the centre toward its
            // record, 1001 outside, filed under class 4, walks back to its record's point
            let world = || {
                let mut t = let_out_world();
                t.object_mut(100).unwrap().1.town_bell.flag = true;
                inside_tc(&mut t, 1, 1000, VILLAGER);
                walker(&mut t, 1, 1001, VILLAGER, [5.5, 5.5]);
                for (id, point) in [(1000, [4.5, 4.5]), (1001, [3.5, 3.5])] {
                    let v = &mut t.object_mut(id).unwrap().1.town_bell;
                    v.flag = true;
                    v.work = crate::orders::town_bell::BackToWork {
                        point,
                        ..crate::orders::town_bell::BackToWork::NONE
                    };
                }
                tracked(&mut t, VILLAGER, &[1000, 1001]);
                t
            };
            let mut t = world();
            t.all_back_to_work(player_id(1), object_id(100)).unwrap();
            assert!(t.object(100).unwrap().1.town_bell.flag);
            let v = t.object(1000).unwrap().1;
            assert_eq!((v.life.inside, v.town_bell.flag), (None, false));
            let u = t.object(1001).unwrap().1;
            assert_eq!(
                (
                    u.actions.move_to().copied().and_then(|m| m.goal),
                    u.town_bell.flag
                ),
                (Some([3.5, 3.5, 0.0]), false)
            );
            // another player's command does nothing without the game's cheats
            let mut t = world();
            t.all_back_to_work(player_id(2), object_id(100)).unwrap();
            assert_eq!(t.object(1000).unwrap().1.life.inside, Some(100));
        }
    }

    #[test]
    fn back_to_work_takes_the_first_record_else_the_second_which_moves_up() {
        // FUN_1412035a0: the first record's object wins; the records stay
        let first = work(10, Some(ActionKind::Gather), [5.0, 6.0]);
        let second = work(8, Some(ActionKind::Build), [1.0, 2.0]);
        let mut bell = TownBell {
            flag: false,
            work: first,
            before_building: second,
        };
        assert_eq!(bell.back_to_work(&object), (object(10), first));
        assert_eq!((bell.work, bell.before_building), (first, second));
        // the first's object gone: the second's object, and the second becomes the first
        let gone = work(11, Some(ActionKind::Gather), [5.0, 6.0]);
        bell.work = gone;
        assert_eq!(bell.back_to_work(&object), (object(8), second));
        assert_eq!(
            (bell.work, bell.before_building),
            (second, BackToWork::NONE)
        );
        // neither has an object: the first record as it is, its targets kept even when a built
        // building ended it
        let ended = work(7, Some(ActionKind::Build), [5.0, 6.0]);
        let mut bell = TownBell {
            flag: false,
            work: ended,
            before_building: work(12, Some(ActionKind::Gather), [1.0, 2.0]),
        };
        assert_eq!(bell.back_to_work(&object), (None, ended));
        assert_eq!(bell.work, ended);
        // ... or the second when the first has no point (−1, −1 on both axes)
        let pointless = work(11, Some(ActionKind::Gather), [-1.0, -1.0]);
        let before = work(12, Some(ActionKind::Gather), [1.0, 2.0]);
        let mut bell = TownBell {
            flag: false,
            work: pointless,
            before_building: before,
        };
        assert_eq!(bell.back_to_work(&object), (None, before));
        // one axis at −1 is a point
        let half = work(11, Some(ActionKind::Gather), [-1.0, 3.0]);
        bell.work = half;
        assert_eq!(bell.back_to_work(&object), (None, half));
        assert_eq!((bell.work, bell.before_building), (half, before));
    }
}
