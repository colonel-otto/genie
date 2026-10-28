//! QUEUE (0x81): units of one master at the player's buildings.

use crate::GenieError;
use crate::commands::UnitList;
use crate::world::World;

/// QUEUE (0x81, the player's queue `FUN_141614450`, the building's `FUN_1411cbc40`): `amount`
/// units of master `unit` at one building with the train button `button` ([`World::queue`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueueCommand {
    /// The player who queues.
    pub player: usize,
    /// The buildings the packet lists (the count at `+0x4`, the ids from `+0x14`).
    pub buildings: UnitList,
    /// The master to train (0x712 for the villager).
    pub unit: i16,
    /// How many.
    pub amount: u16,
    /// `button_location_id`: the train button, −1 for none.
    pub button: i32,
}

impl QueueCommand {
    /// `do_command`'s check refuses a player the world lacks and a count past the packet's ids
    /// ([`World::read_units`]); then the queue at the buildings ([`World::queue`]).
    pub(crate) fn run(&self, w: &mut World) -> Result<(), GenieError> {
        let Some(listed) = w.read_units(self.player, &self.buildings) else {
            return Ok(());
        };
        w.queue(self, listed)
    }
}
