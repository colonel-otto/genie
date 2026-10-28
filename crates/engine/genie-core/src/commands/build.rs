//! BUILD (0x66, `FUN_140e7a400`): a player's foundation and the builders it sends to it.

use crate::GenieError;
use crate::world::World;

/// BUILD (0x66, `FUN_140e7a400`): the player places a foundation of `master` at `position` (state
/// 0, hit points 1), with the unit AI the factory gives its master. The player pays the master's cost, and each builder, in packet order, takes the
/// master it builds with and starts a Build action (`World::masters`). The slice takes the
/// placement as given: the recording's game accepted it.
#[derive(Clone, Debug, PartialEq)]
pub struct BuildCommand {
    /// The player who places it.
    pub player: usize,
    /// The foundation's master.
    pub master: i16,
    /// Where it is placed.
    pub position: [f32; 3],
    /// The units told to build it, in packet order.
    pub builders: Vec<i32>,
    /// `+0x1e`: queued (a builder with a unit AI takes it as an order in its queue).
    pub queued: bool,
    /// `+0x1f` is 0: a builder with a unit AI holding an order not ready takes it as an order
    /// too, and the queue's orders become ready after (`FUN_140c42b60`).
    pub commits: bool,
    /// `+0x1c`: the frame the foundation's standing sprite takes once built (its `+0xc54`,
    /// [`crate::objects::unit::Unit::build_frame`]).
    pub frame: u8,
}

impl BuildCommand {
    /// `do_command`'s case checks the builders first (another player's become −1; a human's
    /// BUILD left without one is refused). A farm on rice terrain takes its rice master; then the
    /// pending debit (`FUN_141630d10` with 0), the builders' test, the placement test and the
    /// payment: a refused BUILD places nothing. The foundation keeps the command's frame
    /// (`+0xc54`) before its builders get their tasks.
    pub(crate) fn run(&self, w: &mut World) -> Result<(), GenieError> {
        let player = self.player;
        let Some(builders) = w.command_check(player, &self.builders) else {
            return Ok(());
        };
        let master = w.farm_on_rice_terrain(self.master, self.position);
        w.debit_listed(player, master)?;
        if !w.build_has_builder(player, master, &builders)? {
            return Ok(());
        }
        if !w.build_allowed(player, master, [self.position[0], self.position[1]])? {
            return Ok(());
        }
        let (id, position) = w.found(player, master, self.position)?;
        if let Some((_, u)) = w.object_mut(id) {
            u.build_frame = self.frame;
        }
        w.task_builders(
            player,
            id,
            master,
            position,
            &builders,
            (self.queued, self.commits),
        )
    }
}
