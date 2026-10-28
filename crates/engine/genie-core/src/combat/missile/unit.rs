//! A missile's flight: where it ends, what it hits and where it lands.

use crate::GenieError;
use crate::combat::victims::victim_of;
use crate::combat::volley::Fired;
use crate::math::rng::Stream;
use crate::objects::lifecycle::death_parts::forget_resource;
use crate::objects::unit::{Unit, missed_passes};
use crate::unit_ai::ACTION_EXECUTED;
use crate::world::context::Context;

impl Unit {
    /// The missile action's update (slot 6, `FUN_140a9e6e0`). First its target and the object it
    /// was fired from: one gone from the world's table is dropped; a dead target (above state 2
    /// but 6, `FUN_1411944f0`) is let go (slot 0x68 with none, `FUN_140a7a230`: slot 0x180).
    /// Then state 4 flies ([`crate::combat::missile::MissileAction::fly`]): the step (`+0x7dc`) moves the
    /// missile (`FUN_14113fb60`: off the map its hit points go, slot 0xd8; the position setter,
    /// slot 0x58, holds it on the map and puts it on the ground by its fly mode once it is no
    /// farther than 0.1 from its point, [`crate::combat::missile::grounded`]); the objects its step
    /// crossed meet it ([`Unit::missile_hits`]); a missile whose height is the ground's lands
    /// ([`Unit::missile_lands`]). A dead shooter's weapons going to the missile
    /// (`FUN_14113e170`), the sprite's pitch facet and the projectile types 1 and 2 are not
    /// modelled.
    pub(crate) fn fly(
        &mut self,
        c: &mut Context<'_>,
        others: &mut [Self],
    ) -> Result<(), GenieError> {
        let Some(mut missile) = self.missile.take() else {
            return Ok(());
        };
        let mut action = self.actions.take_missile();
        let flown = self.fly_missile(&mut missile, &mut action, c, others);
        self.actions.set_missile(action);
        self.missile = Some(missile);
        flown
    }

    /// [`Unit::fly`]'s first step: the missile's target and the object it was fired from, one
    /// gone from the world's table dropped, a dead target (above state 2 but 6) let go.
    ///
    /// # Errors
    ///
    /// A target or a shooter in the world the slice cannot read, and a dead shooter (its
    /// weapons going to the missile, `FUN_14113e170`).
    pub(crate) fn missile_ends(
        &self,
        action: &mut crate::combat::missile::MissileAction,
        c: &mut Context<'_>,
        others: &[Self],
    ) -> Result<(), GenieError> {
        let dead = |state: u8| 2 < state && state != 6;
        let target_state = match c.victim.as_deref() {
            Some(v) if v.id == action.target => Some(v.state),
            _ => None,
        };
        let shooter_state = others
            .iter()
            .find(|u| u.id == action.fired_from)
            .map(|u| u.life.state);
        if action.target != -1 && target_state.is_none() {
            if c.spatial.get(action.target).is_some() {
                return Err(GenieError::unsupported(
                    "a missile's target the slice cannot read",
                ));
            }
            action.target = -1;
        }
        if action.fired_from != -1 && shooter_state.is_none() {
            if c.spatial.get(action.fired_from).is_some() {
                return Err(GenieError::unsupported(
                    "a missile's shooter the slice cannot read",
                ));
            }
            action.fired_from = -1;
        }
        if action.target != -1 && target_state.is_some_and(dead) {
            c.releases.push((action.target, self.releaser(c)));
            action.target = -1;
        }
        if action.fired_from != -1 && shooter_state.is_some_and(dead) {
            return Err(GenieError::unsupported(
                "a missile whose shooter died (its weapons, FUN_14113e170)",
            ));
        }
        Ok(())
    }

    pub(crate) fn fly_missile(
        &mut self,
        missile: &mut crate::combat::missile::Missile,
        slot: &mut Option<crate::combat::missile::MissileAction>,
        c: &mut Context<'_>,
        others: &mut [Self],
    ) -> Result<(), GenieError> {
        let master =
            c.masters
                .and_then(|m| m.get(self.life.master_id))
                .ok_or(GenieError::unsupported(
                    "a missile of a master the slice does not have",
                ))?;
        let stats = master.missile_stats.ok_or(GenieError::unsupported(
            "a missile whose master has no flight",
        ))?;
        let Some(action) = slot.as_mut() else {
            return Ok(());
        };
        self.missile_ends(action, c, others)?;
        match action.state {
            crate::combat::missile::FLYING => {}
            crate::combat::missile::DONE => {
                return Err(GenieError::unsupported(
                    "a missile action's update in state 1",
                ));
            }
            _ => {
                return Err(GenieError::unsupported(
                    "a missile action in a state but 1 and 4",
                ));
            }
        }
        let (step, flying) = action.fly(c.dt_seconds, missile.max_range, &stats)?;
        missile.step = step;
        missile.flying = flying;
        let old = self.position;
        let to = [old[0] + step[0], old[1] + step[1], old[2] + step[2]];
        let [w, h] = c.spatial.map;
        if to[0] < 0.0 || w as f32 <= to[0] || to[1] < 0.0 || h as f32 <= to[1] {
            self.life.hp = 0.0;
        }
        let mover = self
            .body
            .mover_mut()
            .ok_or(GenieError::unsupported("a missile without its moving part"))?;
        let mut at = mover.set_position(to, [w, h]);
        let ground = c
            .spatial
            .terrain
            .as_ref()
            .ok_or(GenieError::unsupported(
                "a missile without the map's terrain",
            ))?
            .ground(at[0], at[1])?;
        at[2] = crate::combat::missile::grounded(at[2], ground, flying, stats.fly_mode);
        mover.mark_move(at, c.world_time);
        self.position = at;
        c.spatial.set_position(self.id, at);
        if matches!(stats.projectile_type, 0..=2) {
            let fired = Fired {
                charge: missile.charge,
                by: missile.fired_from,
            };
            self.missile_hits(action, &stats, fired, old, c, others)?;
        }
        if ground == at[2] {
            self.missile_lands(missile, slot, &stats, c, others)?;
        }
        Ok(())
    }

    /// `FUN_140a9e6e0`'s hits (projectile types 0 to 2): the objects whose obstruction boxes,
    /// grown by the missile's radius on x, its step from `old` crossed (`FUN_14113f740` ->
    /// `FUN_1411570c0`: the query `FUN_141051900` with flags 0x10, the missile itself ignored, the
    /// ends held inside the map) and that lie at its height ([`crate::combat::missile::meets_height`]).
    /// Each one the last step did not meet, while the missile still flies: hit mode 0 hits its
    /// target alone; hit mode 1 (a miss) an object of another player that is no ally's and no
    /// missile, but not one of Gaia's above type 15 or one below the missile's blast level unless
    /// it is the target. A hit is the shooter's ([`Unit::missile_strike`]); unless the master's
    /// vanish mode has bit 0 or a shield blocked it, the missile ends (`set_state` 1: slot 0xd8,
    /// its hit points 0). The step's objects become the met ones (`+0x88`) when there are any. A
    /// missed missile's hit on another object than its target, hit mode 2 and the Hussite
    /// wagon's case (`+0x90`) are not modelled.
    pub(crate) fn missile_hits(
        &mut self,
        action: &mut crate::combat::missile::MissileAction,
        stats: &crate::combat::missile::MissileStats,
        fired: Fired,
        old: [f32; 3],
        c: &mut Context<'_>,
        others: &mut [Self],
    ) -> Result<(), GenieError> {
        if !(0.0 < stats.radius[0] && 0.0 < stats.radius[1]) {
            return Ok(());
        }
        let blast_level = c
            .masters
            .and_then(|m| m.get(self.life.master_id))
            .map_or(0, |m| m.blast_level);
        let met = self.missile_meets(stats, old, c)?;
        for &(id, player, kind, level) in &met {
            if action.state == crate::combat::missile::DONE {
                break;
            }
            if action.met.contains(&id) {
                continue;
            }
            match action.hit_mode {
                0 => {
                    if id != action.target {
                        continue;
                    }
                    let result = self.missile_strike(action, fired, c, others)?;
                    action.result = result;
                    if stats.vanish_mode & 1 == 0 && result != 2 {
                        self.end_missile(action);
                    }
                }
                1 => {
                    if missed_passes(c, (id, player, kind, level), action.target, blast_level) {
                        continue;
                    }
                    if id != action.target {
                        return Err(GenieError::unsupported(
                            "a missed missile meeting another object than its target",
                        ));
                    }
                    let result = self.missile_strike(action, fired, c, others)?;
                    action.result = result;
                    // the master's dying sprite plays there (a sprite of its own, not held)
                    if (stats.vanish_mode & 1 != 0 || result == 2) && blast_level <= level {
                        continue;
                    }
                    self.end_missile(action);
                }
                _ => return Err(GenieError::unsupported("a missile of hit mode 2")),
            }
        }
        if !met.is_empty() {
            action.met = met.iter().map(|m| m.0).collect();
        }
        Ok(())
    }

    /// [`Unit::missile_hits`]'s objects the missile meets on its step from `old`: the step held
    /// on the map ([`crate::combat::missile::QUERY_EDGE`] short of its far edges), the records its sweep
    /// grown by the missile's radius crosses (the manager's walk query with the missile's player,
    /// the missile itself ignored), of those each whose height the missile's meets
    /// ([`crate::combat::missile::meets_height`]): its id, player, master type and blast defence.
    ///
    /// # Errors
    ///
    /// A frozen object the sweep may cross, and an object of a master the slice does not have.
    pub(crate) fn missile_meets(
        &self,
        stats: &crate::combat::missile::MissileStats,
        old: [f32; 3],
        c: &Context<'_>,
    ) -> Result<Vec<(i32, usize, u8, u8)>, GenieError> {
        let [w, h] = c.spatial.map;
        let hold = |v: f32, size: i32| {
            if v < 0.0 {
                0.0
            } else if size as f32 <= v {
                size as f32 - crate::combat::missile::QUERY_EDGE
            } else {
                v
            }
        };
        let at = self.position;
        let from = [hold(old[0], w), hold(old[1], h)];
        let to = [hold(at[0], w), hold(at[1], h)];
        let rect = [
            from[0].min(to[0]),
            from[1].min(to[1]),
            from[0].max(to[0]),
            from[1].max(to[1]),
        ];
        let grow = stats.radius[0];
        c.spatial.check_known_records([
            rect[0] - grow,
            rect[1] - grow,
            rect[2] + grow,
            rect[3] + grow,
        ])?;
        let crossed = c
            .spatial
            .walk_hits(from, to, rect, grow, 1.0, Some(c.player), &[self.id])?;
        let mut met = Vec::new();
        for id in crossed {
            let Some(p) = c.spatial.get(id) else {
                continue;
            };
            let m = c
                .every_masters
                .get(p.player)
                .and_then(|m| m.get(p.master))
                .ok_or(GenieError::unsupported(
                    "an object a missile meets of a master the slice does not have",
                ))?;
            if crate::combat::missile::meets_height(p.position[2], m.height, at[2], stats.radius[2])
            {
                met.push((id, p.player, m.kind, m.blast_defense));
            }
        }
        Ok(met)
    }

    /// The missile action's `set_state(1)` (`FUN_140a9de70`): the missile's slot 0xd8
    /// (`FUN_14118f580`), its hit points to 0.
    pub(crate) const fn end_missile(&mut self, action: &mut crate::combat::missile::MissileAction) {
        action.state = crate::combat::missile::DONE;
        self.life.hp = 0.0;
    }

    /// The shooter's hit for its missile (`FUN_140a9e6e0`: slot 0x860 of the object it was fired
    /// from, `FUN_1412115e0`, at the missile's place with 1 and the missile flag, the action's
    /// origin as the hit's, whose height the elevation factor reads): [`crate::combat::attack::strike`] on
    /// the target, the executed action posted to the shooter's unit AI, the shooter's own part
    /// written back; its result. A shooterless missile hits on its own ([`Unit::own_strike`]). A
    /// charge projectile's own hit, a shooter with an area effect (its trample or blast) and a
    /// retaliation killing the shooter are not modelled; a building shooter's elevation factor
    /// is its own (`FUN_1411cc890`, its relics: [`crate::combat::attack::strike`]).
    pub(crate) fn missile_strike(
        &mut self,
        action: &crate::combat::missile::MissileAction,
        fired: Fired,
        c: &mut Context<'_>,
        others: &mut [Self],
    ) -> Result<u8, GenieError> {
        if fired.charge {
            return Err(GenieError::unsupported("a charge projectile's own hit"));
        }
        if action.fired_from == -1 {
            return self.own_strike(action, fired.by, c);
        }
        let shooter = others
            .iter_mut()
            .find(|u| u.id == action.fired_from)
            .ok_or(GenieError::unsupported(
                "a missile's shooter outside its list",
            ))?;
        let master = c
            .masters
            .and_then(|m| m.get(shooter.life.master_id))
            .ok_or(GenieError::unsupported(
                "a missile's shooter of a master the slice does not have",
            ))?;
        if 0.0 < master.area_effect {
            return Err(GenieError::unsupported(
                "a missile of a shooter with an area effect",
            ));
        }
        let terrain = c.spatial.terrain.as_ref();
        let mut own = victim_of(shooter, master, c.attributes, c.player, terrain, c.mode_680);
        let victim = c
            .victim
            .as_deref_mut()
            .filter(|v| v.id == action.target)
            .ok_or(GenieError::unsupported(
                "a missile's target the slice does not hold live",
            ))?;
        let hit = crate::combat::attack::Hit {
            at: self.position,
            from_z: Some(action.origin.position[2]),
            missile: true,
            chance: master.hit_chance,
        };
        let object = c.streams.get(Stream::Object);
        let struck = crate::combat::attack::strike(&mut own, victim, &mut c.blasted, hit, object)?;
        if let Some(target) = struck.executed {
            shooter.post_own(ACTION_EXECUTED, [600, target, 0])?;
        }
        if shooter.hit_back(own)?.is_some() {
            return Err(GenieError::unsupported(
                "a retaliation killing a missile's shooter",
            ));
        }
        Ok(struck.result)
    }

    /// A shooterless missile's own hit (`FUN_140a9e6e0`: slot 0x860 of the missile,
    /// `FUN_141084230`, at its place): [`crate::combat::attack::strike`] with the missile as the attacker,
    /// its own master's weapons and hit chance (its origin names no master), and no elevation
    /// factor (the missile's slot 0x1f0, `FUN_14113e550`: 1 for an origin without an owner); the
    /// damage's notify comes from what it was fired from (`+0x998`, [`crate::combat::damage`]). The
    /// executed action goes to the missile's unit AI, which it has none of; what a retaliation
    /// does to it is written back. A missile with an area effect (its blast) or whose origin
    /// names an owner is not modelled.
    pub(crate) fn own_strike(
        &mut self,
        action: &crate::combat::missile::MissileAction,
        fired_from: i32,
        c: &mut Context<'_>,
    ) -> Result<u8, GenieError> {
        if action.origin.owner != -1 {
            return Err(GenieError::unsupported(
                "a shooterless missile with an origin (its elevation, FUN_14113e550)",
            ));
        }
        let master =
            c.masters
                .and_then(|m| m.get(self.life.master_id))
                .ok_or(GenieError::unsupported(
                    "a missile of a master the slice does not have",
                ))?;
        if 0.0 < master.area_effect {
            return Err(GenieError::unsupported(
                "a shooterless missile with an area effect",
            ));
        }
        let terrain = c.spatial.terrain.as_ref();
        let mut own = victim_of(self, master, c.attributes, c.player, terrain, c.mode_680);
        // the missile's own part is out of the unit while it flies
        own.source.fired_from = fired_from;
        let victim = c
            .victim
            .as_deref_mut()
            .filter(|v| v.id == action.target)
            .ok_or(GenieError::unsupported(
                "a missile's target the slice does not hold live",
            ))?;
        let hit = crate::combat::attack::Hit {
            at: self.position,
            from_z: None,
            missile: true,
            chance: master.hit_chance,
        };
        let object = c.streams.get(Stream::Object);
        let struck = crate::combat::attack::strike(&mut own, victim, &mut c.blasted, hit, object)?;
        if self.hit_back(own)?.is_some() {
            return Err(GenieError::unsupported("a retaliation killing a missile"));
        }
        Ok(struck.result)
    }

    /// `FUN_140a9e6e0`'s landing: a missile still flying toward a target it did not meet lands on
    /// it when it lies on the target's box ([`crate::combat::missile::on_target`]) and its master's hit
    /// mode is 0 or 2, or 1 against another player's target: the shooter's hit
    /// ([`Unit::missile_strike`]), which ends it unless a shield blocked it. A missile that still
    /// flies after that hits the ground: unless its last hit was blocked, the blast of the object
    /// it was fired from there (slot 0x848, `FUN_14107f690`: nothing without an area effect; one
    /// is not modelled); it ends (`set_state` 1) and, unless its master's vanish mode has bit 1, is
    /// destroyed (slot 0xd0, `FUN_14106c420`: its action list goes, which lets its target and
    /// shooter go; `FUN_14118f1e0`: its hit points 0 and state 7).
    pub(crate) fn missile_lands(
        &mut self,
        missile: &crate::combat::missile::Missile,
        slot: &mut Option<crate::combat::missile::MissileAction>,
        stats: &crate::combat::missile::MissileStats,
        c: &mut Context<'_>,
        others: &mut [Self],
    ) -> Result<(), GenieError> {
        let fired = Fired {
            charge: missile.charge,
            by: missile.fired_from,
        };
        let charge = fired.charge;
        let Some(action) = slot.as_mut() else {
            return Ok(());
        };
        if action.state != crate::combat::missile::DONE
            && action.target != -1
            && !action.met.contains(&action.target)
        {
            let v = c
                .victim
                .as_deref()
                .filter(|v| v.id == action.target)
                .ok_or(GenieError::unsupported(
                    "a missile's target the slice does not hold live",
                ))?;
            let (target_at, target_radius, target_player) = (v.position, v.radius, v.player);
            let mode = stats.hit_mode;
            if (matches!(mode, 0 | 2) || (mode == 1 && c.player != target_player))
                && crate::combat::missile::on_target(
                    self.position,
                    stats.radius,
                    target_at,
                    target_radius,
                )
            {
                let result = self.missile_strike(action, fired, c, others)?;
                action.result = result;
                if result != 2 {
                    self.end_missile(action);
                }
            }
        }
        if action.state == crate::combat::missile::DONE {
            return Ok(());
        }
        if action.result != 2 {
            let blaster = if !charge && action.fired_from != -1 {
                others
                    .iter()
                    .find(|u| u.id == action.fired_from)
                    .map(|u| u.life.master_id)
            } else {
                Some(self.life.master_id)
            };
            let area = blaster
                .and_then(|id| c.masters.and_then(|m| m.get(id)))
                .map(|m| m.area_effect)
                .ok_or(GenieError::unsupported(
                    "a landed missile's blast of a master the slice does not have",
                ))?;
            if 0.0 < area {
                return Err(GenieError::unsupported("a landed missile's blast"));
            }
        }
        self.end_missile(action);
        if stats.vanish_mode & 2 == 0 {
            if let Some(a) = slot.take() {
                let by = self.releaser(c);
                c.releases.extend(
                    [a.target, a.fired_from]
                        .into_iter()
                        .filter(|&t| t != -1)
                        .map(|t| (t, by)),
                );
            }
            if self.life.state < 7 {
                if self.life.inside.is_some() {
                    return Err(GenieError::unsupported(
                        "a destroyed object inside another (slot 0x1a8)",
                    ));
                }
                let before = self.life.state;
                self.life.hp = 0.0;
                self.life.goto_sleep = false;
                self.life.state = 7;
                c.spatial.sight_state(self.id, 7);
                if before < 4 {
                    let master = c.masters.and_then(|m| m.get(self.life.master_id));
                    forget_resource(c.spatial, master, self.id, self.position);
                }
            }
        }
        Ok(())
    }
}
