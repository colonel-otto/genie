//! GATE (0x72): a player's gate lock.

use crate::GenieError;
use crate::world::World;

/// GATE (0x72, `do_command` case 0x72): the player's building `id` toggles its gate lock
/// ([`World::gate`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GateCommand {
    /// The player.
    pub player: usize,
    /// Its building.
    pub id: i32,
}

impl GateCommand {
    /// Its handler ([`World::gate`]).
    pub(crate) fn run(&self, w: &mut World) -> Result<(), GenieError> {
        w.gate(self.player, self.id)
    }
}
