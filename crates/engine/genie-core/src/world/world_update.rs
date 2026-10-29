//! The world update's parts ([`crate::world::schedule`] runs them): the command phase, the
//! moving objects, the players' relations and each player's objects' updates.

use crate::GenieError;
use crate::commands::Command;
use crate::objects::lifecycle::seconds;
use crate::objects::unit::{Outcome, Unit};
use crate::ownership::capture::Captured;
use crate::unit_ai::unit::AiResume;
use crate::world::World;

impl World {
    /// One world update of `dt` game milliseconds without commands.
    pub fn tick(&mut self, dt: u32) {
        self.tick_with(dt, &[]);
    }

    /// An update's start up to the end of its command phase (`BaseWorld::update` up to
    /// `RGE_Command__do_commands`, before `FUN_141552990`): the update's count and the world's
    /// time go on by `dt`, the streams take the frame seed and `commands` run; no player updates.
    /// For a check of the commands against the game's state there (the tick oracle's
    /// `snapshot_at` at `0x141552990`); the schedule's first step
    /// ([`crate::world::schedule::WorldStep::Commands`]).
    pub fn command_phase(&mut self, dt: u32, commands: &[Command]) {
        self.ticks += 1;
        self.world_time = self.world_time.wrapping_add(dt);
        self.dt_seconds = seconds(dt);
        self.dt = dt;
        self.streams.seed_all(self.frame_seed);
        self.sync_moving();
        for command in commands {
            tracing::debug!(target: crate::trace::UPDATE, ?command, "command");
            let stopped: Vec<bool> = self
                .players
                .iter()
                .map(|p| p.unsupported.is_some())
                .collect();
            self.run(command);
            // a player the command stopped: its error names the command
            for (p, was) in self.players.iter_mut().zip(stopped) {
                if let (false, Some((_, _, why))) = (was, p.unsupported.as_mut()) {
                    let text = format!("{command:?}");
                    let name = text.split([' ', '{', '(']).next().unwrap_or_default();
                    *why = why.clone().with("command", name);
                }
            }
        }
        self.sync_moving();
    }

    /// The walks read units `ids`' gaits ([`Unit::gait`]) from the spatial index: after a change
    /// to their movers outside their own update (a group update's), each takes its unit's again
    /// (a unit the world does not hold, none).
    ///
    /// [`Unit::gait`]: crate::objects::unit::Unit::gait
    pub(crate) fn refresh_gaits(&mut self, ids: impl IntoIterator<Item = i32>) {
        for id in ids {
            let gait = self.object(id).and_then(|(_, u)| u.gait());
            self.spatial.set_gait(id, gait);
        }
    }

    /// Tell the world which objects move now and how the players stand to each other, for the
    /// walk's query ([`Spatial::walk_hits`]), as each update does first: for a look at the
    /// world between updates.
    ///
    /// [`Spatial::walk_hits`]: crate::map::spatial::Spatial::walk_hits
    pub fn sync_world(&mut self) {
        self.sync_moving();
    }

    /// Tell the world which objects move now and how the players stand to each other, for the
    /// walk's query ([`Spatial::walk_hits`]).
    ///
    /// [`Spatial::walk_hits`]: crate::map::spatial::Spatial::walk_hits
    pub(super) fn sync_moving(&mut self) {
        for p in &self.players {
            for u in p.objects.iter().chain(&p.sleeping) {
                self.spatial.set_gait(u.id, u.gait());
                self.spatial.set_annex(u.id, u.annex_of);
            }
        }
        if self.spatial.relations.is_empty() {
            self.spatial.relations = self.players.iter().map(|p| p.relations.clone()).collect();
            self.spatial.task_0x99 = self
                .masters
                .iter()
                .map(|m| {
                    m.iter()
                        .filter(|m| m.tasks.iter().any(|t| t.action_type == 0x99))
                        .map(|m| m.id)
                        .collect()
                })
                .collect();
        }
    }

    /// Some player's attribute 204 is not 0, which a building's completion reads: the players'
    /// attributes as they stand.
    #[must_use]
    pub(crate) fn attribute_204(&self) -> bool {
        self.players
            .iter()
            .any(|q| q.attributes.get(204).is_some_and(|&v| v != 0.0))
    }
}

impl World {
    /// The updates of player `p`'s objects in its list's order (`FUN_141166550`): each one's
    /// step and what follows it ([`World::step_object`]), then its filing by the step
    /// ([`World::file_step`]). An object whose update is not modelled freezes, and so do all live
    /// objects of a player a command stopped; dead objects still run their state steps, which
    /// draw nothing and depend on nothing the stop changed.
    pub(super) fn update_objects(&mut self, p: usize, (dt, turn): (u32, Option<u32>)) {
        // the loop's count, which goes on past each object but one that leaves
        let mut i = 0;
        // a unit whose update stopped before its unit AI for its work order runs that part
        // again once the work is given ([`Unit::ai_part`]), at the slot its step left it in
        let mut resume: Option<(AiResume, usize)> = None;
        while resume.is_some() || i < self.players[p].objects.len() {
            let (again, slot) = resume.take().map_or((None, i), |(r, k)| (Some(r), k));
            let player = &self.players[p];
            let stopped = player.stopped || player.objects[slot].stopped;
            if stopped && player.objects[slot].life.is_alive() {
                self.spatial.freeze(player.objects[slot].id);
                i += 1;
                continue;
            }
            let (id, master) = (player.objects[slot].id, player.objects[slot].life.master_id);
            let _object =
                tracing::trace_span!(target: crate::trace::UPDATE, "object", id, master).entered();
            let stepped = self.step_object((p, slot), (dt, turn, true), again, Unit::update);
            match stepped {
                Ok(s) => match (s.outcome, s.resume, s.after) {
                    (Outcome::Stay, Some(ai_resume), _) => resume = Some((ai_resume, s.slot)),
                    // the unit AI's rest after the order on a target it took, which the unit's
                    // update runs again from there ([`crate::unit_ai::UnitAi::after_order`])
                    (Outcome::Stay, None, Some(after)) => {
                        let ai_resume = AiResume {
                            captured: None,
                            leaves: Outcome::Stay,
                            after: Some(after),
                        };
                        resume = Some((ai_resume, s.slot));
                    }
                    (outcome, ..) => i += usize::from(self.file_step(p, s.slot, outcome)),
                },
                Err(e) => {
                    self.freeze_object(p, (i, id), e);
                    i += 1;
                }
            }
        }
    }

    /// Files object `i` of player `p` by the step it finished (the list's loop,
    /// `FUN_141166550`): one that leaves goes from its list, a sleeper to the sleeping list, a
    /// captured building or unit to its new owner. Returns whether the loop's count goes on: not
    /// past one that leaves (the game's loop runs its count again, the list's last object in the
    /// slot), past any other (a sleeper's slot holds the last object too, whose update waits
    /// for the next tick).
    pub(super) fn file_step(&mut self, p: usize, i: usize, outcome: Outcome) -> bool {
        let player = &mut self.players[p];
        match outcome {
            Outcome::Leave => {
                let gone = player.objects.swap_remove(i);
                if let Err(e) = self.leave_world(p, &gone) {
                    crate::trace::player_unsupported(self.ticks, p, &e);
                    let _ = self.players[p]
                        .unsupported
                        .get_or_insert((self.ticks, i, e));
                }
                // the list's last object took its place
                false
            }
            Outcome::Sleep => {
                // the list's remove (FUN_1411664a0) puts its last object in the sleeper's place,
                // and the list's loop (0x141166550) goes on past it: that object's update waits
                // for the next tick; the player's remove and add (slots 0x2d8, 0x2d0) file it last
                // in the tracker
                let asleep = player.objects.swap_remove(i);
                let k = crate::players::tracker::key(
                    self.masters.get(p),
                    &self.lines,
                    asleep.life.master_id,
                );
                player.tracker.remove(asleep.id, k);
                player.tracker.add(asleep.id, k);
                player.sleeping.push(asleep);
                true
            }
            Outcome::Captured(Captured { new, building, .. }) if building => {
                // a building's slot 0x70 (FUN_1411cd860), whole: its annexes, its lists' moves
                // and its new owner's parts
                let id = player.objects[i].id;
                let watched = player.watched.iter().any(|w| w.id == id);
                if let Err(e) = self.building_changes_hands(p, id, new, watched) {
                    crate::trace::player_unsupported(self.ticks, p, &e);
                    let _ = self.players[p]
                        .unsupported
                        .get_or_insert((self.ticks, i, e));
                }
                true
            }
            Outcome::Captured(taken) => {
                self.hand_over_units(p, i, taken);
                true
            }
            Outcome::Stay => true,
        }
    }

    /// The units of player `p` a takeover in object `i`'s update captured go to their new owner:
    /// the object itself when the new owner has its master (`main`), and the units taken with it
    /// (`also`). The old owner's list loses them
    /// (`FUN_1411664a0`, the last in each place; the loop goes on past the first) and the new
    /// owner's gains them (`FUN_141616200`), each with its attributes
    /// ([`World::hand_over_attributes`]).
    fn hand_over_units(&mut self, p: usize, i: usize, taken: Captured) {
        let Captured {
            new, main, also, ..
        } = taken;
        let player = &mut self.players[p];
        let mut moved = Vec::new();
        if main {
            moved.push(player.objects.swap_remove(i));
        }
        for id in also {
            if let Some(k) = player.objects.position_of(id) {
                moved.push(player.objects.swap_remove(k));
            }
        }
        let mut failed = None;
        for u in &moved {
            if let Err(e) = self.hand_over_attributes(p, new, u) {
                let _ = failed.get_or_insert(e);
            }
        }
        if let Some(e) = failed {
            crate::trace::player_unsupported(self.ticks, p, &e);
            let _ = self.players[p]
                .unsupported
                .get_or_insert((self.ticks, i, e));
        }
        // each one's slot 0x70: the old owner's remove (slot 0x2d8), the new owner's add (slot
        // 0x2d0), each tracker by its own masters
        for u in &moved {
            let m = u.life.master_id;
            let old = crate::players::tracker::key(self.masters.get(p), &self.lines, m);
            self.players[p].tracker.remove(u.id, old);
            let k = crate::players::tracker::key(self.masters.get(new), &self.lines, m);
            self.players[new].tracker.add(u.id, k);
        }
        self.players[new].objects.extend(moved);
    }

    /// Object `id` of player `p`, whose step at the loop's count `i` failed with `e`, freezes:
    /// it stays in its list, stopped, its error in the player's freezes, and the player keeps
    /// its first error with the object's slot (the count, once the step took it from the list).
    fn freeze_object(&mut self, p: usize, (i, id): (usize, i32), e: GenieError) {
        let player = &mut self.players[p];
        let slot = player.objects.position_of(id);
        let e = match slot {
            Some(k) => player.objects[k].in_context(e),
            None => e.with("object", id),
        };
        crate::trace::object_freezes(self.ticks, p, id, &e);
        player.freezes.push((self.ticks, id, e.clone()));
        let first = (self.ticks, slot.unwrap_or(i), e);
        let _ = player.unsupported.get_or_insert(first);
        if let Some(k) = slot {
            player.objects[k].stopped = true;
        }
        self.spatial.freeze(id);
    }
}

/// `FUN_141302540`: the game mode (options `+0x34`) is Scenario (3), Campaign (4), King of the
/// Hill (5) or Co-op Campaign (15), by aoc-mgz's game types.
#[must_use]
pub const fn scenario_mode(game_mode: i32) -> bool {
    matches!(game_mode, 3 | 4 | 5 | 15)
}

/// `FUN_141302450`: the game mode's descriptor (`DAT_143f6b330[mode]`, the first for a mode above
/// 16) has `+0x9c` 1, as `FUN_1412d99e0` makes them (`FUN_1412d9360`'s fifth argument): all but
/// the scenario modes 3, 4, 5, 14 and 15.
#[must_use]
pub const fn standard_mode(game_mode: i32) -> bool {
    !matches!(game_mode, 3 | 4 | 5 | 14 | 15)
}
