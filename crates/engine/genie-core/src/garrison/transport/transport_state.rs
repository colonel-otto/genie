//! A Transport's state: the steps of its update from the sail to the unload.

use crate::GenieError;

/// A Transport's state (`+0x18`, set by `set_state`): its update switches on it. The bytes without
/// a state have no case, and no step modelled here sets them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TransportState {
    /// 0: made, not started (the constructor's).
    New = 0,
    /// 1: done; the list removes it.
    Done = 1,
    /// 2: idle: the update does nothing.
    Idle = 2,
    /// 4: sailing to unload.
    SailToUnload = 4,
    /// 6: unloading.
    Unload = 6,
    /// 11: sailing.
    Sail = 11,
    /// 13 ("Failed"): the update tells the unit AI, idles and returns 3.
    Failed = 13,
}

impl TransportState {
    /// The state of a byte of the save or the game's memory.
    ///
    /// # Errors
    ///
    /// A byte that names no state of a Transport.
    pub fn of_byte(value: u8) -> Result<Self, GenieError> {
        Ok(match value {
            0 => Self::New,
            1 => Self::Done,
            2 => Self::Idle,
            4 => Self::SailToUnload,
            6 => Self::Unload,
            11 => Self::Sail,
            13 => Self::Failed,
            _ => {
                return Err(
                    GenieError::invalid_input("no state of a Transport").with("state", value)
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

impl std::fmt::Display for TransportState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.get())
    }
}

impl genie_diff::Diff for TransportState {
    fn diff(&self, other: &Self, d: &mut genie_diff::Differ) {
        d.equal(self, other);
    }
}

#[cfg(test)]
mod tests {
    use crate::garrison::transport::TransportState;

    /// Each state reads back from its byte; the bytes without a state are errors.
    #[test]
    fn its_states_read_back_from_their_bytes() {
        for b in 0..=255_u8 {
            match TransportState::of_byte(b) {
                Ok(s) => assert_eq!(s.get(), b),
                Err(_) => assert!(matches!(b, 3 | 5 | 7..=10 | 12 | 14..), "{b}"),
            }
        }
        assert_eq!(TransportState::Failed.to_string(), "13");
    }
}
