//! The per-tick skeleton the unit-AI slice needs: the frame-seed chain and the unit-AI turn.
//!
//! 158041 `BaseWorld::update` (`0x14154fc70`), in order: advance the world time by dt,
//! `SeedAll(frame seed)`, the player loop (each player's time slot `world+0x18c[p]` gains dt; only the
//! turn player `world+0x188` runs the full unit-AI update), `s1 = World.rand15`, `SeedAll(s1)`, zero
//! the turn player's slot and pass the turn on, then the next frame seed `= World.rand15`. The slice
//! assumes no other World draw runs in a tick (no triggers, XS or unit groups).
//!
//! A player's update walks its live list in order (`RGE_Object_List::update`, `0x141166550`). Each
//! object runs its static state step ([`Lifecycle`]), then its actions (the livestock default
//! action), then its unit AI. An object whose update returns 1 leaves: the last object takes its
//! slot and updates there in the same tick.

use crate::artifact::Artifact;
use crate::lifecycle::{Lifecycle, seconds};
use crate::rng::{Stream, Streams};
use crate::unit_ai::{UnitAi, Unsupported};

/// What the slice tracks of one object.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Unit {
    /// The object's id (`+0x8`), which names it across ticks.
    pub id: i32,
    pub life: Lifecycle,
    pub ai: Option<UnitAi>,
    /// The livestock default action, the object's top action.
    pub artifact: Option<Artifact>,
}

/// What a tick's update of one object needs from the world.
struct Context<'a> {
    /// The tick's dt in milliseconds, and in seconds (`world+0x164`).
    dt: u32,
    dt_seconds: f32,
    /// The ticks so far, this one included.
    tick: u32,
    /// The owner's accumulated time when it has the turn: the full unit-AI update runs.
    turn: Option<u32>,
    world_time: u32,
    computer: bool,
    ai: &'a mut crate::rng::Well512,
}

impl Unit {
    /// One update; true when the object leaves its list.
    fn update(&mut self, c: &mut Context<'_>) -> Result<bool, Unsupported> {
        let leaves = self.life.update(c.dt_seconds)?;
        if !self.life.is_alive() {
            // RGE_Action_Object::update deletes a dead object's action list
            self.artifact = None;
        }
        // The default action exists from tick 1 on, but its first update is in tick 2: fitted to
        // 16 checkpoints of idle players with livestock (workspace FD-008).
        if let Some(artifact) = self.artifact.as_mut().filter(|_| c.tick > 1) {
            artifact.update(c.dt);
        }
        if let (Some(dt), Some(ai)) = (c.turn, self.ai.as_mut().filter(|_| self.life.is_alive())) {
            ai.update_idle(dt, c.world_time, c.computer, c.ai)?;
        }
        Ok(leaves)
    }
}

/// One player's objects in live-list order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Player {
    pub objects: Vec<Unit>,
    /// `player+0x10 == 3`.
    pub computer: bool,
    /// The first update this slice could not model, with the tick (1-based) and object index.
    pub unsupported: Option<(u32, usize, Unsupported)>,
}

/// World state the slice carries from tick to tick. All of it is in the header save.
#[derive(Clone, Debug, PartialEq)]
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
            if player.unsupported.is_some() {
                continue;
            }
            let mut c = Context {
                dt,
                dt_seconds: seconds(dt),
                tick: self.ticks,
                turn: (p == self.player_turn).then_some(self.player_time[p]),
                world_time: self.world_time,
                computer: player.computer,
                ai: self.streams.get(Stream::Ai),
            };
            let mut i = 0;
            while i < player.objects.len() {
                match player.objects[i].update(&mut c) {
                    Ok(true) => {
                        player.objects.swap_remove(i);
                    }
                    Ok(false) => i += 1,
                    Err(e) => {
                        player.unsupported = Some((self.ticks, i, e));
                        break;
                    }
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
    use crate::lifecycle::{Class, Lifecycle, MasterData};

    fn player(objects: Vec<Option<UnitAi>>) -> Player {
        Player {
            objects: objects
                .into_iter()
                .map(|ai| Unit {
                    ai,
                    ..Unit::default()
                })
                .collect(),
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
            t.players[0].objects[0]
                .ai
                .as_ref()
                .unwrap()
                .last_retarget_time,
            0
        );
        assert_eq!(
            t.players[1].objects[0]
                .ai
                .as_ref()
                .unwrap()
                .last_retarget_time,
            10
        );
        t.tick(10);
        t.tick(10);
        // turns 2 (no objects), then 0; back at player 1
        assert_eq!(t.player_turn, 1);
        assert_eq!(t.player_time, [0, 20, 10]);
        assert_eq!(
            t.players[0].objects[0]
                .ai
                .as_ref()
                .unwrap()
                .last_retarget_time,
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
    fn artifacts_of_every_player_start_updating_in_the_second_tick() {
        let animal = |gaia| Unit {
            artifact: Some(Artifact::new(gaia)),
            ..Unit::default()
        };
        let players = vec![
            Player {
                objects: vec![animal(true)],
                ..Player::default()
            },
            Player {
                objects: vec![animal(false)],
                ..Player::default()
            },
        ];
        let mut t = Turns::new(0, 1, 0, vec![0, 0], players);
        let state = |t: &Turns, p: usize| t.players[p].objects[0].artifact.unwrap().state;
        t.tick(50);
        assert_eq!((state(&t, 0), state(&t, 1)), (6, 6));
        t.tick(50);
        assert_eq!((state(&t, 0), state(&t, 1)), (10, 10));
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

    fn dead_building() -> Unit {
        Unit {
            id: 1,
            ai: Some(building()),
            life: Lifecycle {
                class: Class::Animated,
                hp: 0.0,
                master: Some(MasterData::default()),
                ..Lifecycle::default()
            },
            ..Unit::default()
        }
    }

    /// A live building whose retarget timer falls due in the fourth tick of 10 ms (world time 40).
    fn live(id: i32) -> Unit {
        Unit {
            id,
            ai: Some(UnitAi {
                retarget_timer: 40,
                ..building()
            }),
            ..Unit::default()
        }
    }

    fn ids(t: &Turns) -> Vec<i32> {
        t.players[0].objects.iter().map(|o| o.id).collect()
    }

    fn retarget(t: &Turns) -> Vec<u32> {
        t.players[0]
            .objects
            .iter()
            .map(|o| o.ai.as_ref().unwrap().retarget_timer)
            .collect()
    }

    #[test]
    fn a_removed_object_is_replaced_by_the_last_one_which_updates_in_the_same_tick() {
        // RGE_Object_List::update: the dead building leaves in its fourth update; the last object
        // takes its slot and updates there, so it draws before the object behind the slot
        let mut t = Turns::new(
            0,
            29910,
            0,
            vec![0],
            vec![Player {
                objects: vec![dead_building(), live(2), live(3)],
                ..Player::default()
            }],
        );
        for _ in 0..4 {
            t.tick(10);
        }
        assert_eq!(ids(&t), [3, 2]);
        // the same ticks with the two live buildings in that order from the start: their timers
        // fall due in the fourth tick, so its draws depend only on the order
        let mut reordered = Turns::new(
            0,
            29910,
            0,
            vec![0],
            vec![Player {
                objects: vec![live(3), live(2)],
                ..Player::default()
            }],
        );
        for _ in 0..4 {
            reordered.tick(10);
        }
        assert_eq!(retarget(&t), retarget(&reordered));
    }

    #[test]
    fn a_dead_objects_unit_ai_draws_nothing() {
        let one = |objects| {
            let mut t = Turns::new(
                0,
                29910,
                0,
                vec![0],
                vec![Player {
                    objects,
                    ..Player::default()
                }],
            );
            t.tick(10);
            t
        };
        let with_dead = one(vec![dead_building(), live(2)]);
        let alone = one(vec![live(2)]);
        assert_eq!(with_dead.players[0].objects[0].life.state, 5);
        assert_eq!(
            with_dead.players[0].objects[0].ai,
            dead_building().ai,
            "the dead building's timers stay"
        );
        assert_eq!(
            with_dead.players[0].objects[1].ai,
            alone.players[0].objects[0].ai
        );
    }
}
