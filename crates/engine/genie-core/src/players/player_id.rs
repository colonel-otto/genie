//! A player's number as one type: the player a command comes from or names, and the index of
//! the world's players.

use crate::GenieError;
use getset::CopyGetters;

/// A player's number: 0 for Gaia, then the players in their order. A command's byte 1 holds
/// the player who sent it, a comm player the game looks up whatever its value
/// (`FUN_140e6b210`; the slice takes comm and world players as equal), so every byte is one.
/// The world's players are in this order ([`PlayerId::index`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, CopyGetters)]
pub struct PlayerId(
    /// Its number, as a command's byte holds it.
    #[getset(get_copy = "pub")]
    u8,
);

impl PlayerId {
    /// Its place in the world's players.
    #[must_use]
    pub fn index(self) -> usize {
        usize::from(self.0)
    }
}

impl From<u8> for PlayerId {
    /// The player of number `number`, as a command's byte holds it.
    fn from(number: u8) -> Self {
        Self(number)
    }
}

impl TryFrom<usize> for PlayerId {
    type Error = GenieError;

    /// The player at `index` of the world's players.
    ///
    /// # Errors
    ///
    /// An index past a byte, which no command names.
    fn try_from(index: usize) -> Result<Self, GenieError> {
        u8::try_from(index).map(Self::from).map_err(|_| {
            GenieError::invalid_input("a player past a command's byte").with("player", index)
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::ErrorKind;
    use crate::players::PlayerId;

    /// A player's index is its number, and the index converts back.
    #[test]
    fn a_player_s_index_is_its_number() {
        for number in [0, 8, u8::MAX] {
            let p = PlayerId::from(number);
            assert_eq!((p.get(), p.index()), (number, usize::from(number)));
            assert_eq!(PlayerId::try_from(p.index()), Ok(p));
        }
    }

    /// An index past a byte is no player.
    #[test]
    fn an_index_past_a_byte_is_no_player() {
        assert_eq!(
            PlayerId::try_from(256_usize).map_err(|e| e.kind()),
            Err(ErrorKind::InvalidInput)
        );
    }
}
