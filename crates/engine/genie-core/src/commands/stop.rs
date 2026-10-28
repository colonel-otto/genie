//! STOP (0x01): the player's units stop.

use crate::GenieError;
use crate::world::World;

/// STOP (0x01, `FUN_14161e200`): the units `ids` (at most 60, the decoder's) stop
/// ([`World::stop_command`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StopCommand {
    /// The player whose units stop.
    pub player: usize,
    /// The units.
    pub ids: Vec<i32>,
}

impl StopCommand {
    /// Its handler ([`World::stop_command`]).
    pub(crate) fn run(&self, w: &mut World) -> Result<(), GenieError> {
        w.stop_command(self.player, &self.ids)
    }
}
