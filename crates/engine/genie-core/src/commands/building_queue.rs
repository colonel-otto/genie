//! `BUILDING_COMMAND_QUEUE` (0x2d): a step the player's objects queue.

use crate::GenieError;
use crate::world::World;

/// `BUILDING_COMMAND_QUEUE` (0x2d, `0x140e64310`): the objects `ids` (at most a human player's
/// 60) queue a step to `target` (−1 for none) or `point` ([`World::command_queue`]).
#[derive(Clone, Debug, PartialEq)]
pub struct BuildingQueueCommand {
    /// The player.
    pub player: usize,
    /// The objects of the packet.
    pub ids: Vec<i32>,
    /// The object the step is on, −1 for none.
    pub target: i32,
    /// Its point.
    pub point: [f32; 2],
}

impl BuildingQueueCommand {
    /// Its handler ([`World::command_queue`]).
    pub(crate) fn run(&self, w: &mut World) -> Result<(), GenieError> {
        w.command_queue(self.player, &self.ids, self.target, self.point)
    }
}
