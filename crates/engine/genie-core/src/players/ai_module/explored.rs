//! The explored-tile hook (`WorldPlayer` slot 0x88, `FUN_1416397b0`): a computer player's map
//! tells its AI module of a tile it explored for the first time.

use crate::map::Tile;
use crate::players::ai_module::AiModule;
use crate::vision::visibility::Visibility;
use crate::world::World;

impl AiModule {
    /// `FUN_140b8ab60` on the information module: a tile on the map goes into the path map as 0.
    pub fn explored_tile(&mut self, x: i32, y: i32) {
        let info = &mut self.info;
        if 0 <= x && x < info.width && 0 <= y && y < info.height {
            info.path_map.set(Tile::new(x, y), 0);
        }
    }
}

impl World {
    /// Slot 0x88 for every tile a computer player's map explored since the last call, in order:
    /// the visible map queues them ([`Visibility::take_ai_tiles`])
    /// and the AI module is the only reader, so the queue empties before any read of it.
    pub(crate) fn apply_explored_tiles(&mut self) {
        let Some(tiles) = self.spatial.sight.as_mut().map(Visibility::take_ai_tiles) else {
            return;
        };
        for (player, pair) in tiles {
            let (x, y) = (pair.x(), pair.y());
            if let Some(module) = self.ai.get_mut(player) {
                module.explored_tile(x, y);
            }
        }
    }

    /// The first part of the players' sight the slice does not model, once the AI modules have
    /// the tiles the sight explored.
    pub(crate) fn take_sight_unsupported(&mut self) -> Option<crate::GenieError> {
        self.apply_explored_tiles();
        self.spatial.take_sight_unsupported()
    }
}
