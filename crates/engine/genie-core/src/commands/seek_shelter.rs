//! `SEEK_SHELTER` (0x2c): the player's villagers seek shelter.

use crate::GenieError;
use crate::world::World;

/// `SEEK_SHELTER` (0x2c, `FUN_140e6a3a0`): the villagers of `ids` (at most 60, the decoder's)
/// seek shelter in the player's buildings ([`World::seek_shelter`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeekShelterCommand {
    /// The player whose units seek shelter.
    pub player: usize,
    /// The units.
    pub ids: Vec<i32>,
}

impl SeekShelterCommand {
    /// Its handler ([`World::seek_shelter`]).
    pub(crate) fn run(&self, w: &mut World) -> Result<(), GenieError> {
        w.seek_shelter(self.player, &self.ids)
    }
}
