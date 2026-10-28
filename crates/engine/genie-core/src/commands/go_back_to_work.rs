//! `GO_BACK_TO_WORK` (0x80, 0x86): the player's buildings let their villagers back out.

use crate::GenieError;
use crate::world::World;

/// `GO_BACK_TO_WORK` (0x80 with one object, `+4`; 0x86 with a list, its count at `+4`, the ids
/// from `+8`): the player's buildings let their villagers back out to work
/// ([`World::go_back_to_work`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GoBackToWorkCommand {
    /// The player.
    pub player: usize,
    /// Its buildings.
    pub ids: Vec<i32>,
}

impl GoBackToWorkCommand {
    /// Its handler ([`World::go_back_to_work`]).
    pub(crate) fn run(&self, w: &mut World) -> Result<(), GenieError> {
        w.go_back_to_work(self.player, &self.ids)
    }
}
