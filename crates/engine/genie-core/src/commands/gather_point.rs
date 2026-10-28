//! SET GATHER POINT (0x78): the gather point of the player's buildings.

use crate::GenieError;
use crate::world::World;

/// SET GATHER POINT (0x78, `FUN_140e7e9a0`): each of the player's objects of `ids` that takes a
/// gather point (its master garrisons, `+0x5d4`, or its player trains a unit there,
/// `FUN_141625a20`'s count with no object, [`World::trainable_count`]) takes `point` and `target`
/// with its master, or with `own` itself (`FUN_1411c7ff0`); a target of −1 at (−1, −1) clears it,
/// and its unit AI's command queue empties (slot 0x210). The ids of other players' objects are
/// left out (the game setting `+0x1074` that lets them is not modelled), as is the exists byte's
/// value 0xff (`+0xc30`, which keeps the target).
#[derive(Clone, Debug, PartialEq)]
pub struct GatherPointCommand {
    /// The player whose objects take it.
    pub player: usize,
    /// The objects of the packet.
    pub ids: Vec<i32>,
    /// The point.
    pub point: [f32; 2],
    /// The object the point is on, −1 for none.
    pub target: i32,
    /// That object's master.
    pub target_master: i16,
    /// The building itself is the gather point (`+0x18`).
    pub own: bool,
}

impl GatherPointCommand {
    /// Its handler ([`World::gather_point`]).
    pub(crate) fn run(&self, w: &mut World) -> Result<(), GenieError> {
        w.gather_point(self)
    }
}
