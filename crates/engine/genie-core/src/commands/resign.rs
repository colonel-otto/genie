//! RESIGN (0x0b): a player resigns or drops.

use crate::GenieError;
use crate::world::World;

/// RESIGN (0x0b, `FUN_140d25af0`): the comm player `player` resigns, or drops (`dropped`)
/// ([`World::resign`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResignCommand {
    /// The comm player (the world player of the same number in the slice's games).
    pub player: usize,
    /// A drop from the game, not a resignation.
    pub dropped: bool,
}

impl ResignCommand {
    /// Its handler ([`World::resign`]).
    pub(crate) fn run(&self, w: &mut World) -> Result<(), GenieError> {
        w.resign(self.player, self.dropped)
    }
}
