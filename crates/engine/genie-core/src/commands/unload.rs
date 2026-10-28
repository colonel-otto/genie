//! UNLOAD (0x6f): units let out of the player's buildings and transports.

use crate::GenieError;
use crate::commands::UnitList;
use crate::map::Point;
use crate::players::PlayerId;
use crate::world::World;

/// UNLOAD (0x6f, `FUN_14161f410`): the units the packet lists (the count at `+0x4`, the ids
/// from `+0x19`; a building, the units inside it; a unit inside one, itself) let units out
/// ([`World::unload`]) toward `point` (−1, −1 for none), those `flag` and `unit_type` pick
/// (`FUN_14118a610`); `sound` (`+0x18`) plays the building's.
#[derive(Clone, Debug, PartialEq)]
pub struct UnloadCommand {
    /// The player.
    pub player: PlayerId,
    /// Its objects.
    pub units: UnitList,
    /// Where they go.
    pub point: Point,
    /// Which units leave (`FUN_14118a610`'s switch).
    pub flag: u8,
    /// The unit type or object the flag reads.
    pub unit_type: i32,
    /// `+0x18`.
    pub sound: bool,
}

impl UnloadCommand {
    /// The check (`FUN_140e6f620`, [`World::unload_check`]) refuses a count past the packet's
    /// ids; the handler reads the first of them ([`World::read_units`], [`World::unload`]).
    pub(crate) fn run(&self, w: &mut World) -> Result<(), GenieError> {
        let Some(ids) = w.read_units(self.player, &self.units) else {
            return Ok(());
        };
        w.unload(self, ids)
    }
}
