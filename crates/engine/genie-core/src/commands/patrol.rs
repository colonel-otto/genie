//! PATROL (0x15): the player's units patrol through waypoints.

use crate::GenieError;
use crate::world::World;

/// PATROL (0x15, `FUN_140e69cc0`): the units `ids` (the packet's count at `+0x4`, the ids from
/// `+0x5c`) patrol through `waypoints` (1 to 10, their count at `+0xb`, x from `+0xc`, y from
/// `+0x34`); `reset` (`+0x8`) drops their unit AIs' saved patrols ([`World::patrol`]).
#[derive(Clone, Debug, PartialEq)]
pub struct PatrolCommand {
    /// The player whose units patrol.
    pub player: usize,
    /// The units.
    pub ids: Vec<i32>,
    /// The waypoints.
    pub waypoints: Vec<[f32; 2]>,
    /// `+0x8`.
    pub reset: bool,
}

impl PatrolCommand {
    /// Its handler ([`World::patrol`]).
    pub(crate) fn run(&self, w: &mut World) -> Result<(), GenieError> {
        w.patrol(self.player, &self.ids, &self.waypoints, self.reset)
    }
}
