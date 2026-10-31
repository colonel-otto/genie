//! An Explore's state: waiting for a point or walking to it.

use crate::GenieError;

/// An Explore's state (`+0x18`, set by `set_state`, slot 0x80 `0x140a87e30`): its update
/// (`0x140a880c0`) and `set_state` switch on it. The bytes without a state have no case, and no
/// step modelled here sets them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ExploreState {
    /// 0: made, not started (the constructor's).
    New = 0,
    /// 1: done; the list removes it and the unit stands.
    Done = 1,
    /// 2: stopped, its point gone.
    Stopped = 2,
    /// 3: waiting for a point; its unit AI is asked for one.
    Wait = 3,
    /// 11: walking to the point.
    Walk = 11,
    /// 13 ("Failed"): the unit stands; the update tells the unit AI.
    Failed = 13,
    /// 14: ended as 1 and 13 do: the unit stands (a state the interruptible test takes with 0, 1, 3
    /// and 13).
    Ended = 14,
}

impl ExploreState {
    /// The state of a byte of the save or the game's memory.
    ///
    /// # Errors
    ///
    /// A byte that names no state of an Explore.
    pub fn of_byte(value: u8) -> Result<Self, GenieError> {
        Ok(match value {
            0 => Self::New,
            1 => Self::Done,
            2 => Self::Stopped,
            3 => Self::Wait,
            11 => Self::Walk,
            13 => Self::Failed,
            14 => Self::Ended,
            _ => {
                return Err(
                    GenieError::invalid_input("no state of an Explore").with("state", value)
                );
            }
        })
    }

    /// Its number.
    #[must_use]
    pub const fn get(self) -> u8 {
        self as u8
    }
}

impl std::fmt::Display for ExploreState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.get())
    }
}

impl genie_diff::Diff for ExploreState {
    fn diff(&self, other: &Self, d: &mut genie_diff::Differ) {
        d.equal(self, other);
    }
}

#[cfg(test)]
mod tests {
    use crate::movement::explore::ExploreState;

    /// Each state reads back from its byte; the bytes without a state are errors.
    #[test]
    fn its_states_read_back_from_their_bytes() {
        for b in 0..=255_u8 {
            match ExploreState::of_byte(b) {
                Ok(s) => assert_eq!(s.get(), b),
                Err(_) => assert!(matches!(b, 4..=10 | 12 | 15..), "{b}"),
            }
        }
        assert_eq!(ExploreState::Ended.to_string(), "14");
    }
}
