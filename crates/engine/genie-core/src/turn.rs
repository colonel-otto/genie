//! The per-tick skeleton the unit-AI slice needs: the frame-seed chain and the unit-AI turn.
//!
//! 158041 `BaseWorld::update` (`0x14154fc70`), in order: advance the world time by dt,
//! `SeedAll(frame seed)`, the player loop (each player's time slot `world+0x18c[p]` gains dt; only the
//! turn player `world+0x188` runs the full unit-AI update), `s1 = World.rand15`, `SeedAll(s1)`, zero
//! the turn player's slot and pass the turn on, then the next frame seed `= World.rand15`. The slice
//! assumes no other World draw runs in a tick (no triggers, XS or unit groups).

use crate::rng::{Stream, Streams};
use crate::unit_ai::{UnitAi, Unsupported};

/// One player's objects in live-list order; `None` for an object without a unit AI.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Player {
    pub objects: Vec<Option<UnitAi>>,
    /// `player+0x10 == 3`.
    pub computer: bool,
    /// The first update this slice could not model, with the tick (1-based) and object index.
    pub unsupported: Option<(u32, usize, Unsupported)>,
}

/// World state the slice carries from tick to tick. All of it is in the header save.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Turns {
    pub world_time: u32,
    /// `world+0x7d0`.
    pub frame_seed: u32,
    /// `world+0x188`, `player_turn` in the save.
    pub player_turn: usize,
    /// `world+0x18c[p]`, the time each player accumulated since its last turn.
    pub player_time: Vec<u32>,
    pub players: Vec<Player>,
    pub ticks: u32,
    streams: Streams,
}

impl Turns {
    pub fn new(
        world_time: u32,
        frame_seed: u32,
        player_turn: usize,
        player_time: Vec<u32>,
        players: Vec<Player>,
    ) -> Self {
        Turns {
            world_time,
            frame_seed,
            player_turn,
            player_time,
            players,
            ticks: 0,
            streams: Streams::default(),
        }
    }

    /// One world update of `dt` game milliseconds.
    pub fn tick(&mut self, dt: u32) {
        self.ticks += 1;
        self.world_time = self.world_time.wrapping_add(dt);
        self.streams.seed_all(self.frame_seed);
        for (p, player) in self.players.iter_mut().enumerate() {
            self.player_time[p] = self.player_time[p].wrapping_add(dt);
            if p != self.player_turn || player.unsupported.is_some() {
                continue;
            }
            for (i, object) in player.objects.iter_mut().enumerate() {
                let Some(ai) = object else { continue };
                let rng = self.streams.get(Stream::Ai);
                if let Err(e) =
                    ai.update_idle(self.player_time[p], self.world_time, player.computer, rng)
                {
                    player.unsupported = Some((self.ticks, i, e));
                    break;
                }
            }
        }
        let s1 = self.streams.get(Stream::World).rand15();
        self.streams.seed_all(s1);
        self.player_time[self.player_turn] = 0;
        self.player_turn = (self.player_turn + 1) % self.players.len();
        self.frame_seed = self.streams.get(Stream::World).rand15();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn player(objects: Vec<Option<UnitAi>>) -> Player {
        Player {
            objects,
            ..Player::default()
        }
    }

    fn building() -> UnitAi {
        UnitAi {
            ai_type: 17,
            current_order: -1,
            current_action: -1,
            current_target: -1,
            adjusted_idle_timeout: 3000,
            look_around_timeout: 1000,
            retarget_timer: 1,
            ..UnitAi::default()
        }
    }

    #[test]
    fn the_turn_rotates_and_only_the_turn_player_updates() {
        let mut t = Turns::new(
            0,
            29910,
            1,
            vec![0, 0, 0],
            vec![
                player(vec![Some(building())]),
                player(vec![Some(building())]),
                player(vec![]),
            ],
        );
        t.tick(10);
        assert_eq!(t.player_turn, 2);
        assert_eq!(t.player_time, [10, 0, 10]);
        assert_eq!(
            t.players[0].objects[0].as_ref().unwrap().last_retarget_time,
            0
        );
        assert_eq!(
            t.players[1].objects[0].as_ref().unwrap().last_retarget_time,
            10
        );
        t.tick(10);
        t.tick(10);
        // turns 2 (no objects), then 0; back at player 1
        assert_eq!(t.player_turn, 1);
        assert_eq!(t.player_time, [0, 20, 10]);
        assert_eq!(
            t.players[0].objects[0].as_ref().unwrap().last_retarget_time,
            30
        );
    }

    #[test]
    fn the_frame_seed_chains_through_two_world_draws() {
        let mut t = Turns::new(0, 29910, 0, vec![0], vec![player(vec![])]);
        t.tick(10);
        // SeedAll(29910); s1 = World.rand15 = 19550 (see rng); SeedAll(19550); World.rand15
        let mut s = Streams::default();
        s.seed_all(19550);
        assert_eq!(t.frame_seed, s.get(Stream::World).rand15());
    }

    #[test]
    fn an_unsupported_unit_stops_its_player_and_is_recorded() {
        let mut odd = building();
        odd.ai_type = 2;
        let mut t = Turns::new(0, 1, 0, vec![0], vec![player(vec![Some(odd)])]);
        t.tick(10);
        t.tick(10);
        assert_eq!(
            t.players[0].unsupported,
            Some((1, 0, Unsupported("unit-AI type not modelled")))
        );
    }
}
