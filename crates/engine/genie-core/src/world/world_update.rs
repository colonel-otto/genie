//! One world update (`BaseWorld::update`): the command phase, the moving objects and the
//! players' relations, each player's frame and the update with its commands.

use crate::commands::Command;
use crate::math::rng::Stream;
use crate::objects::lifecycle::seconds;
use crate::objects::unit::Step;
use crate::ownership::capture::Captured;
use crate::unit_ai::unit::AiResume;
use crate::world::{PlayerFrame, World};

impl World {
    /// One world update of `dt` game milliseconds without commands.
    pub fn tick(&mut self, dt: u32) {
        self.tick_with(dt, &[]);
    }

    /// An update's start up to the end of its command phase (`BaseWorld::update` up to
    /// `RGE_Command__do_commands`, before `FUN_141552990`): the update's count and the world's
    /// time go on by `dt`, the streams take the frame seed and `commands` run; no player updates.
    /// For a check of the commands against the game's state there (the tick oracle's
    /// `snapshot_at` at `0x141552990`); [`World::tick_with`] runs it first.
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

    /// What every object of player-loop reads of all players (`attribute_97`, the game status,
    /// the computer flags, the dodge and keep-distance chances, the sight takers, attribute 204),
    /// as the players stand when the loop starts.
    pub(super) fn player_frame(&self) -> PlayerFrame {
        let attribute_97: Vec<f32> = self
            .players
            .iter()
            .map(|q| q.attributes.get(97).copied().unwrap_or(0.0))
            .collect();
        let game_status: Vec<u8> = self.players.iter().map(|q| q.game_status).collect();
        let computers: Vec<bool> = self.players.iter().map(|q| q.computer).collect();
        let dodge_chances: Vec<u8> = self.players.iter().map(|q| q.dodge_chance).collect();
        let keep_distance_chances: Vec<u8> = self
            .players
            .iter()
            .map(|q| q.keep_distance_chance)
            .collect();
        let sight_takers: Vec<(bool, bool)> = self
            .players
            .iter()
            .enumerate()
            .map(|(q, player)| {
                let allows = self.allow_gaia_capture.get(q).copied();
                (player.computer, allows.unwrap_or(false))
            })
            .collect();
        let attribute_204 = self
            .players
            .iter()
            .any(|q| q.attributes.get(204).is_some_and(|&v| v != 0.0));
        PlayerFrame {
            attribute_97,
            game_status,
            computers,
            dodge_chances,
            keep_distance_chances,
            sight_takers,
            attribute_204,
        }
    }
}

impl World {
    /// One world update of `dt` game milliseconds, with the commands recorded in it.
    ///
    /// # Panics
    ///
    /// Never: each player updates in turn, so the one it splits off exists.
    pub fn tick_with(&mut self, dt: u32, commands: &[Command]) {
        let _update = tracing::debug_span!(
            target: crate::trace::UPDATE,
            "update",
            tick = self.ticks + 1,
            world_time = self.world_time.wrapping_add(dt)
        )
        .entered();
        self.command_phase(dt, commands);
        // which players' gates are open (BaseWorld::update's flags)
        let mut gates_open = vec![false; self.players.len()];
        // the Empire Wars start's pass, once (the world's +0x6c6 set before the players update)
        let start_work = std::mem::take(&mut self.start_work_due);
        for p in 0..self.players.len() {
            let _player =
                tracing::debug_span!(target: crate::trace::UPDATE, "player", player = p).entered();
            self.player_time[p] = self.player_time[p].wrapping_add(dt);
            let turn = (p == self.player_turn).then_some(self.player_time[p]);
            if let Err(e) = self.open_gates(p, &mut gates_open) {
                self.stop_player(p, e);
            }
            if start_work && let Err(e) = self.start_work(p) {
                self.stop_player(p, e);
            }
            self.pathing.tiles_changed(self.spatial.take_changes());
            // the player's groups (BaseWorld::update at 0x141551796), then its own update (slot
            // 0x100 at 0x1415518f0): the world player's or Gaia's
            // ([`crate::players::world_player`]), then WorldPlayerBase::update: its plans start over
            // (`+0x67c`), then its objects update (FUN_141166550 on its list)
            if let Err(e) = self.update_groups(p) {
                self.stop_player(p, e);
            }
            if let Err(e) = self.update_world_player(p) {
                self.stop_player(p, e);
            }
            self.pathing.reset_plans(p);
            // An object whose update is not modelled freezes, and so do all live objects of a
            // player a command stopped; dead objects still run their state steps, which draw
            // nothing and depend on nothing the stop changed.
            let frame = self.player_frame();
            let mut i = 0;
            // a unit whose update stopped before its unit AI for its work order runs that part
            // again once the work is given ([`Unit::ai_part`])
            let mut resume: Option<AiResume> = None;
            while i < self.players[p].objects.len() {
                let player = &self.players[p];
                let stopped = player.stopped || player.objects[i].stopped;
                if stopped && player.objects[i].life.is_alive() {
                    self.spatial.freeze(player.objects[i].id);
                    resume = None;
                    i += 1;
                    continue;
                }
                let (id, master) = (player.objects[i].id, player.objects[i].life.master_id);
                let _object =
                    tracing::trace_span!(target: crate::trace::UPDATE, "object", id, master)
                        .entered();
                let (result, out, victim) = self.run_object(
                    (p, i),
                    (dt, turn),
                    &frame,
                    resume.take(),
                    |unit, c, objects| unit.update(c, objects),
                );
                let (result, ai_resume, after_order) =
                    self.finish_object((p, i, id), (dt, true), out, victim, result);
                if matches!(result, Ok(Step::Stay)) && ai_resume.is_some() {
                    resume = ai_resume;
                    continue;
                }
                // the unit AI's rest after the order on a target it took, which the unit's
                // update runs again from there ([`crate::unit_ai::UnitAi::after_order`])
                if matches!(result, Ok(Step::Stay))
                    && let Some(after) = after_order
                {
                    resume = Some(AiResume {
                        captured: None,
                        leaves: Step::Stay,
                        after: Some(after),
                    });
                    continue;
                }
                let player = &mut self.players[p];
                match result {
                    Ok(Step::Leave) => {
                        let gone = player.objects.swap_remove(i);
                        if let Err(e) = self.leave_world(p, &gone) {
                            crate::trace::player_unsupported(self.ticks, p, &e);
                            let _ = self.players[p]
                                .unsupported
                                .get_or_insert((self.ticks, i, e));
                        }
                    }
                    Ok(Step::Sleep) => {
                        // the list's remove (FUN_1411664a0) puts its last object in the
                        // sleeper's place, and the list's loop (0x141166550) goes on past it:
                        // that object's update waits for the next tick; the player's remove and
                        // add (slots 0x2d8, 0x2d0) file it last in the tracker
                        let asleep = player.objects.swap_remove(i);
                        let k = crate::players::tracker::key(
                            self.masters.get(p),
                            &self.lines,
                            asleep.life.master_id,
                        );
                        player.tracker.remove(asleep.id, k);
                        player.tracker.add(asleep.id, k);
                        player.sleeping.push(asleep);
                        i += 1;
                    }
                    Ok(Step::Captured(Captured { new, building, .. })) if building => {
                        // a building's slot 0x70 (FUN_1411cd860), whole: its annexes, its
                        // lists' moves and its new owner's parts
                        let id = player.objects[i].id;
                        let watched = player.watched.iter().any(|w| w.id == id);
                        if let Err(e) = self.building_changes_hands(p, id, new, watched) {
                            crate::trace::player_unsupported(self.ticks, p, &e);
                            let _ = self.players[p]
                                .unsupported
                                .get_or_insert((self.ticks, i, e));
                        }
                        i += 1;
                    }
                    Ok(Step::Captured(Captured {
                        new, main, also, ..
                    })) => {
                        // the old owner's list loses them (FUN_1411664a0, the last in each
                        // place; the loop goes on past the first) and the new owner's gains
                        // them (FUN_141616200), each with its attributes
                        // ([`World::hand_over_attributes`])
                        let mut moved = Vec::new();
                        if main {
                            moved.push(player.objects.swap_remove(i));
                        }
                        for id in also {
                            if let Some(k) = player.objects.iter().position(|u| u.id == id) {
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
                        // each one's slot 0x70: the old owner's remove (slot 0x2d8), the new
                        // owner's add (slot 0x2d0), each tracker by its own masters
                        for u in &moved {
                            let m = u.life.master_id;
                            let old =
                                crate::players::tracker::key(self.masters.get(p), &self.lines, m);
                            self.players[p].tracker.remove(u.id, old);
                            let k =
                                crate::players::tracker::key(self.masters.get(new), &self.lines, m);
                            self.players[new].tracker.add(u.id, k);
                        }
                        self.players[new].objects.extend(moved);
                        i += 1;
                    }
                    Ok(Step::Stay) => i += 1,
                    Err(e) => {
                        let e = player.objects[i].in_context(e);
                        crate::trace::object_freezes(self.ticks, p, player.objects[i].id, &e);
                        player
                            .freezes
                            .push((self.ticks, player.objects[i].id, e.clone()));
                        let _ = player.unsupported.get_or_insert((self.ticks, i, e));
                        player.objects[i].stopped = true;
                        self.spatial.freeze(player.objects[i].id);
                        i += 1;
                    }
                }
            }
            // the end of the player's own update: the tiles its map explored give their
            // resources
            if let Err(e) = self.process_new_tiles(p) {
                self.stop_player(p, e);
            }
            // the world player's update after WorldPlayerBase::update: its scout map
            self.world_player_tail(p);
            self.close_gates(p, &mut gates_open);
            // its tile list empties (FUN_141661030)
            if let Some(v) = self.spatial.sight.as_mut() {
                v.clear_new_tiles(p);
            }
        }
        self.spatial.grow_unknown(seconds(dt));
        let s1 = self.streams.get(Stream::World).rand15();
        // every player's ghosts (FUN_141166550 on +0x40), then every player's watched list
        // (FUN_1410905e0); the objects' visibility masks (FUN_14107d280 on +0x80, before the
        // draw) are the registry's, kept as the objects update
        for p in 0..self.players.len() {
            if let Err(e) = self.update_ghosts(p) {
                self.stop_player(p, e);
            }
        }
        for p in 0..self.players.len() {
            if let Err(e) = self.watch_pass(p) {
                self.stop_player(p, e);
            }
        }
        self.streams.seed_all(s1);
        self.player_time[self.player_turn] = 0;
        self.player_turn = (self.player_turn + 1) % self.players.len();
        self.victory_check();
        self.frame_seed = self.streams.get(Stream::World).rand15();
        // setLastRandoms (0x140d3ce20): the multiplayer sync's draws
        let _ = self.streams.set_last_randoms();
        // World::update's last step (0x1415e9266), whatever dt: the world's slot 0x78 with 1
        // (FUN_1415e92b0): the players' masks again and the town centres' flares
        // (FUN_141552df0, [`World::rebuild_vision`]); its UI refresh is not simulation. Masks the
        // slice cannot rebuild leave every player's sight in doubt, as after the load
        if let Err(e) = self.rebuild_vision() {
            for p in 0..self.players.len() {
                self.stop_player(p, e.clone());
            }
        }
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
