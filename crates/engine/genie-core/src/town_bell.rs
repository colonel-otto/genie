//! A villager's town-bell record (`+0x9ac`..`+0x9dc`): what it worked on when an order to a
//! target took it off its work (`FUN_141206a20`), which a later back-to-work order sends it back
//! to, and the flag a garrison order sets (`FUN_1412067f0`). Saved as `mTownBellFlag`,
//! `mBackToWork1` and `mBackToWork2`.

/// One back-to-work record.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BackToWork {
    /// `townBellTargetId` (`+0x9ac`): the top action's target (the pasture of a target in one),
    /// else the target of the unit AI's first own queued order, −1 for none.
    pub target: i32,
    /// `town_bell_target_type` (`+0x9b0`): a Gather's (5) or Hunt's (`0x6e`) target class
    /// (`+0x68`), else −1.
    pub target_type: i32,
    /// `town_bell_action` (`+0x9b4`): the unit AI's current action (`+0x44`).
    pub action: i32,
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
        action: -1,
        target2: -1,
        point: [-1.0, -1.0],
    };
}

/// The unit AI's action of building (`0x25a`), whose record keeps the one before it.
pub const BUILD_ACTION: i32 = 0x25a;

/// The town-bell flag and the two records of a combat object (type 70 and up).
#[derive(Clone, Copy, Debug, PartialEq)]
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
        if now.action == BUILD_ACTION {
            if self.work.action != BUILD_ACTION {
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
const ENDED_ON_A_BUILDING: [i32; 3] = [0x25a, 0x261, 0x265];

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
        && ENDED_ON_A_BUILDING.contains(&record.action)
        && o.building
        && o.class != FARM_CLASS
        && o.built
    {
        out.target = -1;
        out.target2 = -1;
        return (None, out);
    }
    (found, out)
}

/// The farms' class, whose records a built farm does not end.
const FARM_CLASS: i16 = 49;

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

#[cfg(test)]
mod tests {
    use super::*;

    fn work(target: i32, action: i32, point: [f32; 2]) -> BackToWork {
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
            work: work(-1, -1, [29.0, 99.0]),
            ..TownBell::default()
        };
        let building = work(10028, BUILD_ACTION, [30.0, 88.0]);
        bell.record(building);
        assert_eq!(bell.work, building);
        assert_eq!(bell.before_building, work(-1, -1, [29.0, 99.0]));
        let cutting = BackToWork {
            target_type: 15,
            ..work(9365, 613, [24.5, 97.5])
        };
        bell.record(cutting);
        assert_eq!(bell.work, cutting);
        assert_eq!(bell.before_building, BackToWork::NONE);
        // the same work again changes nothing, nor does any work while the flag holds
        bell.before_building = building;
        bell.record(cutting);
        assert_eq!(bell.before_building, building);
        bell.flag = true;
        bell.record(work(1, 2, [3.0, 4.0]));
        assert_eq!(bell.work, cutting);
    }
}
