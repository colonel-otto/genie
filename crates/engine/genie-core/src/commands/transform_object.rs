//! `TRANSFORM_OBJECT` (0x29): an object of the player takes another master.

use crate::GenieError;
use crate::world::World;

/// `TRANSFORM_OBJECT` (0x29, `RGE_Command::do_command`'s case): the player's object `id` takes
/// the master `master` ([`World::transform_object`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransformObjectCommand {
    /// The player.
    pub player: usize,
    /// The object (`+8`).
    pub id: i32,
    /// The new master (`+4`).
    pub master: i16,
}

impl TransformObjectCommand {
    /// Its handler ([`World::transform_object`]).
    pub(crate) fn run(&self, w: &mut World) -> Result<(), GenieError> {
        w.transform_object(self.player, self.id, self.master)
    }
}
