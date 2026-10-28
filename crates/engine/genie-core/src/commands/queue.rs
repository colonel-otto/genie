//! QUEUE (0x81): units of one master at the player's buildings.

use crate::GenieError;
use crate::world::World;

/// QUEUE (0x81, the player's queue `FUN_141614450`, the building's `FUN_1411cbc40`): `amount`
/// units of master `unit` at one building with the train button `button` ([`World::queue`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueueCommand {
    /// The player who queues.
    pub player: usize,
    /// The buildings the packet lists (`+0x14..`, its `num` of them).
    pub buildings: Vec<i32>,
    /// The master to train (0x712 for the villager).
    pub unit: i16,
    /// How many.
    pub amount: u16,
    /// `button_location_id`: the train button, −1 for none.
    pub button: i32,
}

impl QueueCommand {
    /// The queue at the buildings ([`World::queue`]).
    pub(crate) fn run(&self, w: &mut World) -> Result<(), GenieError> {
        w.queue(self)
    }
}
