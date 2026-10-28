//! RESEARCH (0x65): a technology at the player's buildings.

use crate::GenieError;
use crate::world::World;

/// RESEARCH (0x65, `0x140e7e270`): the technology `tech` at the building `building`
/// ([`World::research`]). `count` (`+0x8`) counts the buildings the packet lists, `target`
/// (`+0xc`) is −1 for a research to run, and `shift` (`+0x10`) asks for five at once.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResearchCommand {
    /// The player who researches.
    pub player: usize,
    /// The building of the packet.
    pub building: i32,
    /// `+0x8`: how many buildings the packet lists.
    pub count: i16,
    /// `+0x11..`: the buildings the packet lists.
    pub buildings: Vec<i32>,
    /// `+0xa`: the technology.
    pub tech: i16,
    /// `+0xc`: −1 for a research to run.
    pub target: i32,
    /// `+0x10`: five at once.
    pub shift: bool,
}

impl ResearchCommand {
    /// `do_command`'s case checks the buildings first ([`World::research_check`]), then the
    /// research runs in those left ([`World::research`]).
    pub(crate) fn run(&self, w: &mut World) -> Result<(), GenieError> {
        let Some(buildings) = w.research_check(self) else {
            return Ok(());
        };
        w.research(&Self {
            buildings,
            ..self.clone()
        })
    }
}
