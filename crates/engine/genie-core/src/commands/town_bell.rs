//! `TOWN_BELL` (0x7f): a town centre's bell.

use crate::GenieError;
use crate::world::World;

/// `TOWN_BELL` (0x7f, `do_command` case 0x7f): the town centre `id` rings its owner's bell
/// (`activate`) or ends it ([`World::town_bell`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TownBellCommand {
    /// The player.
    pub player: usize,
    /// The town centre.
    pub id: i32,
    /// Ring (1) or end (0).
    pub activate: bool,
}

impl TownBellCommand {
    /// Its handler ([`World::town_bell`]).
    pub(crate) fn run(&self, w: &mut World) -> Result<(), GenieError> {
        w.town_bell(self.player, self.id, self.activate)
    }
}
