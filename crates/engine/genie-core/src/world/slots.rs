//! The checked reach into the world's player table and the players' object lists.
//!
//! An index into either comes from the world's own places (a loop over the players, a
//! [`crate::orders::Place`], a `position_of`), so a miss is an invariant broken, not an input
//! the recording got wrong: each lookup returns an `unreachable` error that names the index.
//! The trait is implemented for the slice, not for [`crate::world::World`], so a caller keeps
//! the borrow of the one field (`self.players`) and still reads the others.

use crate::GenieError;
use crate::objects::unit::Unit;
use crate::world::player::Player;

/// A player the world does not hold.
const NO_PLAYER: GenieError = GenieError::unreachable("a player the world does not hold");

/// A slot of a player's live or sleeping list that the list does not hold.
const NO_SLOT: GenieError =
    GenieError::unreachable("an object slot the player's list does not hold");

/// Player `p`, and the objects at a position of its lists, as errors where a slice's index
/// would panic.
pub(crate) trait PlayerSlots {
    /// Player `p`.
    ///
    /// # Errors
    ///
    /// A player the table does not hold.
    fn player(&self, p: usize) -> Result<&Player, GenieError>;

    /// Player `p`, to change.
    ///
    /// # Errors
    ///
    /// A player the table does not hold.
    fn player_mut(&mut self, p: usize) -> Result<&mut Player, GenieError>;

    /// The object at position `i` of player `p`'s live list.
    ///
    /// # Errors
    ///
    /// A player or a position the lists do not hold.
    fn live(&self, p: usize, i: usize) -> Result<&Unit, GenieError> {
        self.player(p)?.live(i).map_err(|e| e.with("player", p))
    }

    /// The object at position `i` of player `p`'s live list, to change.
    ///
    /// # Errors
    ///
    /// A player or a position the lists do not hold.
    fn live_mut(&mut self, p: usize, i: usize) -> Result<&mut Unit, GenieError> {
        self.player_mut(p)?
            .live_mut(i)
            .map_err(|e| e.with("player", p))
    }
}

impl PlayerSlots for [Player] {
    fn player(&self, p: usize) -> Result<&Player, GenieError> {
        self.get(p).ok_or_else(|| NO_PLAYER.with("player", p))
    }

    fn player_mut(&mut self, p: usize) -> Result<&mut Player, GenieError> {
        self.get_mut(p).ok_or_else(|| NO_PLAYER.with("player", p))
    }
}

impl Player {
    /// The object at position `i` of the live list.
    ///
    /// # Errors
    ///
    /// A position the list does not hold.
    pub(crate) fn live(&self, i: usize) -> Result<&Unit, GenieError> {
        self.objects.get(i).ok_or_else(|| NO_SLOT.with("index", i))
    }

    /// The object at position `i` of the live list, to change.
    ///
    /// # Errors
    ///
    /// A position the list does not hold.
    pub(crate) fn live_mut(&mut self, i: usize) -> Result<&mut Unit, GenieError> {
        self.objects
            .get_mut(i)
            .ok_or_else(|| NO_SLOT.with("index", i))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A player or a position the table does not hold is an error that names it.
    #[test]
    fn a_missing_player_or_slot_is_an_error_naming_it() {
        let mut players = [Player::default(), Player::default()];
        assert!(players.player(1).is_ok());
        let e = players.player(2).unwrap_err();
        assert_eq!(e.context().collect::<Vec<_>>(), [("player", "2")]);
        assert!(players.player_mut(5).is_err());
        let e = players.live(1, 0).unwrap_err();
        let keys: Vec<_> = e.context().map(|(k, v)| (k, v.to_owned())).collect();
        assert_eq!(
            keys,
            [("index", "0".to_owned()), ("player", "1".to_owned())]
        );
        assert!(players.live_mut(0, 3).is_err());
        assert!(players.player(0).unwrap().live(0).is_err());
    }
}
