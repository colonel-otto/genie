//! An Enter's state: walking to a building or transport, waiting, inside.

use crate::GenieError;

/// An Enter's state (`+0x18`, set by `set_state`, `FUN_140a87320`): its update switches on it. The
/// bytes without a state have no case, and no step modelled here sets them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EnterState {
    /// 1: done: inside; the list removes it.
    Done = 1,
    /// 2: waiting a second before it walks again.
    Wait = 2,
    /// 4: walking to the target (the constructor's).
    Walk = 4,
    /// 11: its target died or is not allied: a quiet `MoveTo` to the meeting point.
    WalkPoint = 11,
    /// 13 ("Failed"): the list removes it.
    Failed = 13,
}

impl EnterState {
    /// The state of a byte of the save or the game's memory.
    ///
    /// # Errors
    ///
    /// A byte that names no state of an Enter.
    pub fn of_byte(value: u8) -> Result<Self, GenieError> {
        Ok(match value {
            1 => Self::Done,
            2 => Self::Wait,
            4 => Self::Walk,
            11 => Self::WalkPoint,
            13 => Self::Failed,
            _ => {
                return Err(GenieError::invalid_input("no state of an Enter").with("state", value));
            }
        })
    }

    /// Its number.
    #[must_use]
    pub const fn get(self) -> u8 {
        self as u8
    }
}

impl std::fmt::Display for EnterState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.get())
    }
}

impl genie_diff::Diff for EnterState {
    fn diff(&self, other: &Self, d: &mut genie_diff::Differ) {
        d.equal(self, other);
    }
}

#[cfg(test)]
mod tests {
    use crate::garrison::EnterState;

    /// Each state reads back from its byte; the bytes without a state are errors.
    #[test]
    fn its_states_read_back_from_their_bytes() {
        for b in 0..=255_u8 {
            match EnterState::of_byte(b) {
                Ok(s) => assert_eq!(s.get(), b),
                Err(_) => assert!(matches!(b, 0 | 3 | 5..=10 | 12 | 14..), "{b}"),
            }
        }
        assert_eq!(EnterState::Failed.to_string(), "13");
    }
}
