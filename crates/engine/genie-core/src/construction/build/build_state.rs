//! A Build's state: the steps of its update from the walk to the finished building.

use crate::GenieError;

/// A Build's state (`+0x18`, set by `set_state`): its update switches on it. The update's
/// default case takes the bytes without a state (0, 5, 9, 10, 12, 14 to 221 and above 224),
/// which no step modelled here sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BuildState {
    /// 1: done; the list removes it.
    Done = 1,
    /// 2: the unit's stop halted the Build (slot 0x38); its next update fails it.
    Halted = 2,
    /// 3: the action waits without a target.
    Idle = 3,
    /// 4: on the way to the foundation (the constructor's).
    Move = 4,
    /// 6: facing the foundation, then placing it.
    Work = 6,
    /// 7: building.
    Place = 7,
    /// 8: the builder carries its load to a drop site.
    Drop = 8,
    /// 11: the walk to the action's point.
    WalkPoint = 11,
    /// 13 ("Failed"): its next update tells its unit AI and halts it.
    Failed = 13,
    /// 222 (`0xde`): the wait for an occupied footprint.
    Wait = 222,
    /// 223 (`0xdf`): off the footprint, to a free tile beside it.
    Clear = 223,
    /// 224 (`0xe0`): the work ended, the building completed.
    Finished = 224,
}

impl BuildState {
    /// The state of a byte of the save or the game's memory.
    ///
    /// # Errors
    ///
    /// A byte that names no state of a Build.
    pub fn of_byte(value: u8) -> Result<Self, GenieError> {
        Ok(match value {
            1 => Self::Done,
            2 => Self::Halted,
            3 => Self::Idle,
            4 => Self::Move,
            6 => Self::Work,
            7 => Self::Place,
            8 => Self::Drop,
            11 => Self::WalkPoint,
            13 => Self::Failed,
            222 => Self::Wait,
            223 => Self::Clear,
            224 => Self::Finished,
            _ => {
                return Err(GenieError::invalid_input("no state of a Build").with("state", value));
            }
        })
    }

    /// Its number.
    #[must_use]
    pub const fn get(self) -> u8 {
        self as u8
    }
}

impl std::fmt::Display for BuildState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.get())
    }
}

impl genie_diff::Diff for BuildState {
    fn diff(&self, other: &Self, d: &mut genie_diff::Differ) {
        d.equal(self, other);
    }
}

#[cfg(test)]
mod tests {
    use crate::construction::build::BuildState;

    /// Each state reads back from its byte; the bytes without a state are errors.
    #[test]
    fn a_build_s_states_read_back_from_their_bytes() {
        for b in 0..=255_u8 {
            match BuildState::of_byte(b) {
                Ok(s) => assert_eq!(s.get(), b),
                Err(_) => assert!(matches!(b, 0 | 5 | 9 | 10 | 12 | 14..=221 | 225..), "{b}"),
            }
        }
        assert_eq!(BuildState::Finished.to_string(), "224");
    }
}
