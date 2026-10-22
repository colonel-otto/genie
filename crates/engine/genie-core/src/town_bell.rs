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

    /// The objects of a test world: a house 7 that is built, a house 8 that is not, a farm 9
    /// that is built, a sheep 10.
    fn object(id: i32) -> Option<RecordTarget> {
        let (building, class, built) = match id {
            7 => (true, 3, true),
            8 => (true, 3, false),
            9 => (true, FARM_CLASS, true),
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
            ..work(10, 0x261, [5.0, 6.0])
        };
        assert_eq!(record_object(shepherd, &object), (object(10), shepherd));
        // a target the world no longer holds: the second target, unless it is below 0
        let gone = BackToWork {
            target2: 10,
            ..work(11, 0x265, [5.0, 6.0])
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
            ..work(-1, 0x25a, [5.0, 6.0])
        };
        assert_eq!(record_object(second, &object), (object(10), second));
        // building, gathering and hunting (mask 0x881 over 0x25a..0x265) on a built building
        // other than a farm end the record: no object, both targets −1 in the copy
        for action in [0x25a, 0x261, 0x265] {
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
        for action in [0x25b, 0x260, 0x262, 0x264, 0x266, 0x259, -1] {
            let r = work(7, action, [5.0, 6.0]);
            assert_eq!(record_object(r, &object), (object(7), r));
        }
    }

    #[test]
    fn back_to_work_takes_the_first_record_else_the_second_which_moves_up() {
        // FUN_1412035a0: the first record's object wins; the records stay
        let first = work(10, 0x261, [5.0, 6.0]);
        let second = work(8, BUILD_ACTION, [1.0, 2.0]);
        let mut bell = TownBell {
            flag: false,
            work: first,
            before_building: second,
        };
        assert_eq!(bell.back_to_work(&object), (object(10), first));
        assert_eq!((bell.work, bell.before_building), (first, second));
        // the first's object gone: the second's object, and the second becomes the first
        let gone = work(11, 0x261, [5.0, 6.0]);
        bell.work = gone;
        assert_eq!(bell.back_to_work(&object), (object(8), second));
        assert_eq!(
            (bell.work, bell.before_building),
            (second, BackToWork::NONE)
        );
        // neither has an object: the first record as it is, its targets kept even when a built
        // building ended it
        let ended = work(7, BUILD_ACTION, [5.0, 6.0]);
        let mut bell = TownBell {
            flag: false,
            work: ended,
            before_building: work(12, 0x261, [1.0, 2.0]),
        };
        assert_eq!(bell.back_to_work(&object), (None, ended));
        assert_eq!(bell.work, ended);
        // ... or the second when the first has no point (−1, −1 on both axes)
        let pointless = work(11, 0x261, [-1.0, -1.0]);
        let before = work(12, 0x261, [1.0, 2.0]);
        let mut bell = TownBell {
            flag: false,
            work: pointless,
            before_building: before,
        };
        assert_eq!(bell.back_to_work(&object), (None, before));
        // one axis at −1 is a point
        let half = work(11, 0x261, [-1.0, 3.0]);
        bell.work = half;
        assert_eq!(bell.back_to_work(&object), (None, half));
        assert_eq!((bell.work, bell.before_building), (half, before));
    }
}
