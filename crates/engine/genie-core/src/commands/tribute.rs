//! GIVE ATTRIBUTES (0xc4): the player's tribute to another.

use crate::GenieError;
use crate::world::World;

/// GIVE ATTRIBUTES (0xc4, `FUN_140e7d040`): the player's tribute to player `to`
/// ([`World::tribute`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TributeCommand {
    /// The player who gives.
    pub player: usize,
    /// The player who receives (`+0x2c`).
    pub to: usize,
    /// The four entries.
    pub gifts: [crate::players::tribute::Gift; 4],
}

impl TributeCommand {
    /// Its handler ([`World::tribute`]).
    pub(crate) fn run(&self, w: &mut World) -> Result<(), GenieError> {
        w.tribute(self.player, self.to, &self.gifts)
    }
}
