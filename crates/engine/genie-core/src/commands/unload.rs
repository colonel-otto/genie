//! UNLOAD (0x6f): units let out of the player's buildings and transports.

use crate::GenieError;
use crate::world::World;

/// UNLOAD (0x6f, `FUN_14161f410`): the units `ids` (a building, the units inside it; a unit
/// inside one, itself) let units out ([`World::unload`]) toward `point` (−1, −1 for none), those
/// `flag` and `unit_type` pick (`FUN_14118a610`); `sound` (`+0x18`) plays the building's.
#[derive(Clone, Debug, PartialEq)]
pub struct UnloadCommand {
    /// The player.
    pub player: usize,
    /// Its objects.
    pub ids: Vec<i32>,
    /// Where they go.
    pub point: [f32; 2],
    /// Which units leave (`FUN_14118a610`'s switch).
    pub flag: u8,
    /// The unit type or object the flag reads.
    pub unit_type: i32,
    /// `+0x18`.
    pub sound: bool,
}

impl UnloadCommand {
    /// Its handler ([`World::unload`]).
    pub(crate) fn run(&self, w: &mut World) -> Result<(), GenieError> {
        w.unload(self)
    }
}
