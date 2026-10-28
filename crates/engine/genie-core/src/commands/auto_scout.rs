//! `AUTO_SCOUT` (0x26): the player's units scout by themselves.

use crate::GenieError;
use crate::world::World;

/// `AUTO_SCOUT` (0x26, `FUN_140e666f0`): each of `ids` (at most 60, the decoder's) takes the auto
/// scout order, or the stop order when it scouts already ([`World::auto_scout`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutoScoutCommand {
    /// The player.
    pub player: usize,
    /// Its units.
    pub ids: Vec<i32>,
}

impl AutoScoutCommand {
    /// Its handler ([`World::auto_scout`]).
    pub(crate) fn run(&self, w: &mut World) -> Result<(), GenieError> {
        w.auto_scout(self.player, &self.ids)
    }
}
