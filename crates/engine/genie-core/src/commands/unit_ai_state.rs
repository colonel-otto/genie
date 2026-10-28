//! `UNIT_AI_STATE` (0x12): the stance of the player's units.

use crate::GenieError;
use crate::world::World;

/// `UNIT_AI_STATE` (0x12, `FUN_140e6a7c0`): the unit AIs of `ids` (at most 60, the decoder's)
/// take the stance `state` ([`World::unit_ai_state`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnitAiStateCommand {
    /// The player whose units take it.
    pub player: usize,
    /// The units.
    pub ids: Vec<i32>,
    /// The stance.
    pub state: u8,
}

impl UnitAiStateCommand {
    /// Its handler ([`World::unit_ai_state`]).
    pub(crate) fn run(&self, w: &mut World) -> Result<(), GenieError> {
        w.unit_ai_state(self.player, &self.ids, self.state)
    }
}
