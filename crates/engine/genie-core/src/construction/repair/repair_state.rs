//! A Repair's state: the steps of its update from the walk to the work.

use crate::GenieError;

/// A Repair's state (`+0x18`, set by `set_state`, `0x140ad9c60`): its update (`0x140adaa20`)
/// and `set_state` switch on it. The update has no case for the other bytes (0, 5, 8, 9, 12 and
/// above 13), and no step modelled here sets them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RepairState {
    /// 1: done; the target is let go and the list removes it.
    Done = 1,
    /// 2: halted (a failed repair's end), the standing sprite.
    Halted = 2,
    /// 3: idle; the unit AI is told to search (notify 0x202).
    Idle = 3,
    /// 4: on the way to the target (the constructor's).
    Move = 4,
    /// 6: turning toward the target.
    Face = 6,
    /// 7: repairing, counted among the target's repairers.
    Work = 7,
    /// 10: waiting a second for the way to the target.
    Wait = 10,
    /// 11: on the way to its point.
    WalkPoint = 11,
    /// 13 ("Failed"): the target cannot be reached; the update tells the unit AI and halts.
    Failed = 13,
}

impl RepairState {
    /// The state of a byte of the save or the game's memory.
    ///
    /// # Errors
    ///
    /// A byte that names no state of a Repair.
    pub fn of_byte(value: u8) -> Result<Self, GenieError> {
        Ok(match value {
            1 => Self::Done,
            2 => Self::Halted,
            3 => Self::Idle,
            4 => Self::Move,
            6 => Self::Face,
            7 => Self::Work,
            10 => Self::Wait,
            11 => Self::WalkPoint,
            13 => Self::Failed,
            _ => {
                return Err(GenieError::invalid_input("no state of a Repair").with("state", value));
            }
        })
    }

    /// Its number.
    #[must_use]
    pub const fn get(self) -> u8 {
        self as u8
    }
}

impl std::fmt::Display for RepairState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.get())
    }
}

impl genie_diff::Diff for RepairState {
    fn diff(&self, other: &Self, d: &mut genie_diff::Differ) {
        d.equal(self, other);
    }
}

#[cfg(test)]
mod tests {
    use crate::construction::repair::RepairState;

    /// Each state reads back from its byte; the bytes without a state are errors.
    #[test]
    fn a_repair_s_states_read_back_from_their_bytes() {
        for b in 0..=255_u8 {
            match RepairState::of_byte(b) {
                Ok(s) => assert_eq!(s.get(), b),
                Err(_) => assert!(matches!(b, 0 | 5 | 8 | 9 | 12 | 14..), "{b}"),
            }
        }
        assert_eq!(RepairState::Wait.to_string(), "10");
    }
}
