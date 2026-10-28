//! PATROL (0x15): the player's units patrol through waypoints.

use crate::GenieError;
use crate::commands::UnitList;
use crate::world::World;

/// PATROL (0x15, `FUN_140e69cc0`): the units the packet lists (the count at `+0x4`, the ids from
/// `+0x5c`) patrol through `waypoints` (1 to 10, their count at `+0xb`, x from `+0xc`, y from
/// `+0x34`); `reset` (`+0x8`) drops their unit AIs' saved patrols ([`World::patrol`]).
#[derive(Clone, Debug, PartialEq)]
pub struct PatrolCommand {
    /// The player whose units patrol.
    pub player: usize,
    /// The units.
    pub units: UnitList,
    /// The waypoints.
    pub waypoints: Vec<[f32; 2]>,
    /// `+0x8`.
    pub reset: bool,
}

impl PatrolCommand {
    /// The check (`FUN_140e55590`) refuses a count past the packet's ids; the handler reads the
    /// first of them ([`World::read_units`], [`World::patrol`]).
    pub(crate) fn run(&self, w: &mut World) -> Result<(), GenieError> {
        let Some(ids) = w.read_units(self.player, &self.units) else {
            return Ok(());
        };
        w.patrol(self.player, ids, &self.waypoints, self.reset)
    }
}
