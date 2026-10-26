//! The forced replay (dev only): the game's state after a tick ([`GameState`]) and the step
//! that forces it on the world.

use crate::GenieError;
use crate::movement::Mover;
use crate::objects::unit::{Unit, same_members};
use crate::turn::Turns;
use crate::turn::world::forget_resource;

/// The game's state after a tick, for the oracle's forced replay (dev only: the oracle reads it
/// from the tick oracle's saves, genie-core never reads the game): the frame seed and every
/// player's live units and resources as the game holds them.
#[derive(Clone, Debug, Default)]
pub struct GameState {
    /// `world+0x7d0`, the frame seed.
    pub frame_seed: u32,
    /// Every player's live objects.
    pub players: Vec<Vec<Unit>>,
    /// Every player's food, wood, stone and gold.
    pub resources: Vec<[f32; 4]>,
    /// Every player's attributes as saved (the first four are the resources), which the oracle
    /// compares; forcing leaves them alone.
    pub attributes: Vec<Vec<f32>>,
    /// The players' pending debits and technology states.
    pub pending_debit: Vec<[f32; 4]>,
    /// Every player's technology states.
    pub techs: Vec<Vec<crate::technology::TechState>>,
    /// Every player's game status and resignation (`+0x89`, `+0x8b`).
    pub statuses: Vec<(u8, bool)>,
    /// The world's game state (`+0x88`).
    pub game_state: u32,
    /// Objects the slice modelled but got wrong this tick: they take the game's state too.
    pub resync: Vec<i32>,
    /// The players' visible maps and the unified map, when the slice's differ.
    pub sight: Option<(Vec<crate::vision::visibility::VisibleMap>, Vec<u64>)>,
    /// `next_reusable_object_id`: the fog ghosts that take reusable ids are not modelled.
    pub next_reusable_id: i32,
    /// The map tiles' object lists in the game's order (`x`, `y`, ids; each tile of two objects
    /// or more), when the oracle dumped them: the order is the game's history, no save holds it.
    pub tile_lists: Option<Vec<(i32, i32, Vec<i32>)>>,
    /// The pool of recycled missiles ([`Recycling::missiles`]), when the oracle dumped it: no
    /// save holds it.
    ///
    /// [`Recycling::missiles`]: crate::turn::Recycling::missiles
    pub missile_pool: Option<Vec<(i32, bool)>>,
    /// The world's pool of recycled ghosts, when the oracle dumped it.
    pub ghost_pool: Option<Vec<i32>>,
    /// The world's unit groups, when the slice's differ.
    pub groups: Option<crate::groups::UnitGroups>,
}

impl Turns {
    /// The objects' inside lists in the game's order (`+0x2f8`, the tick oracle's dump): each
    /// container listed takes the order when it holds the same objects. Returns how many did.
    pub fn force_garrisons(&mut self, lists: &[(i32, Vec<i32>)]) -> usize {
        let mut n = 0;
        for (id, order) in lists {
            let Some(c) = self
                .players
                .iter_mut()
                .flat_map(|p| p.objects.iter_mut().chain(p.sleeping.iter_mut()))
                .find(|u| u.id == *id)
            else {
                continue;
            };
            if same_members(&c.garrison, order) {
                c.garrison.clone_from(order);
                n += 1;
            }
        }
        n
    }

    /// The forced replay's step after a tick: the frame seed is the game's, and so is every
    /// object the slice did not model (it froze, or its player stopped) or got wrong (`resync`),
    /// and the players' sight where it differs: its unit is the game's
    /// (with what the save does not hold, a hunted animal's origin and a boar's flag, kept),
    /// placed where the game has it, and it runs again from the next tick; a frozen object the
    /// game no longer holds goes. A stopped player takes the game's resources and runs again.
    /// The world's next reusable id is the game's (the fog ghosts that take them are not
    /// modelled). A stopped player's pending debit and technologies are the game's too, and so
    /// is the tiles' objects' order where the game's dump has it, and a forced object's inside
    /// list keeps the slice's order while it holds the same objects. Returns
    /// what could not be forced (an object that changed hands or masters, or
    /// that the game made and the slice did not). Not part of a replay: a dev hook to see past a
    /// freeze.
    pub fn force(&mut self, game: &GameState) -> Vec<(usize, i32, GenieError)> {
        let mut notes = Vec::new();
        self.frame_seed = game.frame_seed;
        self.game_state = game.game_state;
        self.recycling.next_id = game.next_reusable_id;
        if let Some(pool) = game.ghost_pool.as_ref() {
            self.recycling.ghosts.clone_from(pool);
        }
        if let Some(pool) = game.missile_pool.as_ref() {
            self.recycling.missiles.clone_from(pool);
        }
        if let Some(groups) = game.groups.as_ref() {
            self.groups.clone_from(groups);
        }
        if let (Some((maps, unified)), Some(sight)) =
            (game.sight.as_ref(), self.spatial.sight.as_mut())
        {
            sight.maps.clone_from(maps);
            sight.unified.clone_from(unified);
        }
        // the objects the slice holds, sorted: forcing only takes away objects the game no
        // longer holds
        let mut known: Vec<i32> = self
            .players
            .iter()
            .flat_map(|q| q.objects.iter().chain(&q.sleeping).map(|u| u.id))
            .collect();
        known.sort_unstable();
        for p in 0..self.players.len() {
            let Some(theirs) = game.players.get(p) else {
                continue;
            };
            let stopped = self.players[p].stopped;
            if stopped {
                if let Some(r) = game.resources.get(p) {
                    self.players[p].resources = *r;
                }
                if let Some(d) = game.pending_debit.get(p) {
                    self.players[p].pending_debit = *d;
                }
                if let Some(t) = game.techs.get(p) {
                    self.players[p].techs.clone_from(t);
                }
                self.players[p].stopped = false;
            }
            if let Some(&(status, resigned)) = game.statuses.get(p) {
                self.players[p].game_status = status;
                self.players[p].resigned = resigned;
            }
            // each update's first unmodelled part is its own: a freeze's would hide a later stop
            self.players[p].unsupported = None;
            let mut i = 0;
            while i < self.players[p].objects.len() {
                let unit = &self.players[p].objects[i];
                let resync = game.resync.contains(&unit.id);
                let (id, frozen) = (unit.id, stopped || unit.stopped || resync);
                // a dead object never freezes, but one that differs takes the game's state too
                // (a felled tree's wood that a frozen gatherer did not take)
                if !frozen || (!unit.life.is_alive() && !resync) {
                    i += 1;
                    continue;
                }
                let Some(g) = theirs.iter().find(|g| g.id == id) else {
                    let elsewhere = game.players.iter().any(|q| q.iter().any(|g| g.id == id));
                    if elsewhere {
                        notes.push((
                            p,
                            id,
                            GenieError::unsupported("a forced object changed hands"),
                        ));
                        i += 1;
                        continue;
                    }
                    let gone = self.players[p].objects.swap_remove(i);
                    if let Err(e) = self.leave_world(p, &gone) {
                        notes.push((p, id, e));
                    }
                    continue;
                };
                if g.life.master_id != unit.life.master_id {
                    notes.push((
                        p,
                        id,
                        GenieError::unsupported("a forced object changed masters"),
                    ));
                    i += 1;
                    continue;
                }
                let mut forced = g.clone();
                // the objects inside it keep the slice's order (the game's history, which no save
                // holds) while they are the same
                if same_members(&unit.garrison, &forced.garrison) {
                    forced.garrison.clone_from(&unit.garrison);
                }
                if let (Some(old), Some(new)) = (unit.ai.as_ref(), forced.ai.as_mut()) {
                    new.hunted = old.hunted;
                    if let (Some(o), Some(n)) = (old.home, new.home.as_mut()) {
                        n.go_home = o.go_home;
                    }
                }
                let (before, state) = (unit.life.state, forced.life.state);
                if forced.position != unit.position {
                    self.spatial.set_position(id, forced.position);
                }
                if state != before {
                    if before <= 2 && 3 <= state {
                        self.spatial.sight_state(id, 3);
                    }
                    if state != 3 {
                        self.spatial.sight_state(id, state);
                    }
                    if before < 4 && 6 < state {
                        let master = self
                            .masters
                            .get(p)
                            .and_then(|m| m.get(forced.life.master_id));
                        forget_resource(&mut self.spatial, master, id, forced.position);
                    }
                }
                self.spatial.unfreeze(id);
                self.spatial.set_gait(id, forced.gait());
                if let Some(placed) = self.spatial.get(id).copied() {
                    let moving = forced.mover.as_ref().is_some_and(Mover::is_moving);
                    self.pathing.moved(id, &placed, moving);
                }
                forced.stopped = false;
                self.players[p].objects[i] = forced;
                i += 1;
            }
            for g in theirs {
                if known.binary_search(&g.id).is_err() {
                    notes.push((
                        p,
                        g.id,
                        GenieError::unsupported("an object the game made and the slice did not"),
                    ));
                }
            }
        }
        if let Some(lists) = game.tile_lists.as_ref() {
            // the tiles whose objects differ keep the slice's order: their objects' places
            // differ, and that is reported
            let _ = self.spatial.force_tile_lists(lists);
        }
        self.pathing.tiles_changed(self.spatial.take_changes());
        notes
    }
}
