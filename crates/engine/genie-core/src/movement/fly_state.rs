//! A Fly's state: a bird resting, choosing its flight and flying.

use crate::GenieError;

/// A Fly's state (`+0x18`, set by `set_state`, `0x140a84a50`): its update switches on it. The bytes
/// without a state have no case, and no step modelled here sets them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FlyState {
    /// 0: made, not started (the constructor's).
    New = 0,
    /// 1: done (a player's bird whose task has `+0x15` set goes here first, the start's caller).
    Done = 1,
    /// 2: resting until its timer runs out.
    Resting = 2,
    /// 3: choosing its flight.
    Choosing = 3,
    /// 4: flying, a `MoveTo` to its point.
    Flying = 4,
}

impl FlyState {
    /// The state of a byte of the save or the game's memory.
    ///
    /// # Errors
    ///
    /// A byte that names no state of a Fly.
    pub fn of_byte(value: u8) -> Result<Self, GenieError> {
        Ok(match value {
            0 => Self::New,
            1 => Self::Done,
            2 => Self::Resting,
            3 => Self::Choosing,
            4 => Self::Flying,
            _ => {
                return Err(GenieError::invalid_input("no state of a Fly").with("state", value));
            }
        })
    }

    /// Its number.
    #[must_use]
    pub const fn get(self) -> u8 {
        self as u8
    }
}

impl std::fmt::Display for FlyState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.get())
    }
}

impl genie_diff::Diff for FlyState {
    fn diff(&self, other: &Self, d: &mut genie_diff::Differ) {
        d.equal(self, other);
    }
}

#[cfg(test)]
mod tests {
    use crate::movement::fly::FlyState;

    /// Each state reads back from its byte; the bytes without a state are errors.
    #[test]
    fn its_states_read_back_from_their_bytes() {
        for b in 0..=255_u8 {
            match FlyState::of_byte(b) {
                Ok(s) => assert_eq!(s.get(), b),
                Err(_) => assert!(matches!(b, 5..), "{b}"),
            }
        }
        assert_eq!(FlyState::Flying.to_string(), "4");
    }
}
