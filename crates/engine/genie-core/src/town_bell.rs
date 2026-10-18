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
