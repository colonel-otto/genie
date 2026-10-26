//! An attack's start, a hunt's and an action's retarget.

use crate::math::rng::Stream;
use crate::order::BUILDING;
use crate::turn::Turns;
use crate::unit_ai::GenieError;

impl Turns {
    /// The work command's Attack (its task of action type 7, the factory `FUN_140a97e00` case 7):
    /// the unit AI takes action 600 (`FUN_140c64b50`) and the target (an annex its main building,
    /// `FUN_141189b20`) at its place (`FUN_140c64c10`); the Attack (`FUN_140a7c210`, sub-action 0:
    /// the task's sprites, the task's range or slot 0x238's for 0) takes its target (slot 0x178);
    /// the notify (`FUN_14106e4e0` kind 9: the last retarget time `+0xbc` takes the world's time,
    /// then order 700, action 600, the target with its class and the work's point); the install
    /// clears the action list and starts the Attack (slot 5, `0x140a7f8c0`: `+0x900` cleared, the
    /// standing sprite, the range test of [`Attack::command`]; a unit that cannot move in reach
    /// goes ready, `set_state(6)`, [`Attack::enter_ready`]; one that may move ends in
    /// `set_state(4)`, whose `MoveTo` starting arrived shows the standing sprite at its speed and
    /// clears `+0x7b8`, `0x140aa29e0`); then the Attack takes the target as
    /// its second target too (`FUN_140a7a190`, slot 0x178). The unit AI's `+0x1d5`, cleared, is
    /// not held; a task range other than 0 is not modelled.
    pub(super) fn start_attack(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
        task: &crate::masters::Task,
        point: [f32; 3],
        clears: bool,
    ) -> Result<(), GenieError> {
        use crate::combat::attack::Strike;
        use crate::movement::move_to::Walk;
        if task.work_range != 0.0 {
            return Err(GenieError::unsupported(
                "an attack task with a range of its own",
            ));
        }
        let (tp, t) = self.object(target).ok_or(GenieError::unsupported(
            "an attack task's target the slice does not hold",
        ))?;
        let tm = self.master_of(tp, t.life.master_id)?;
        let aimed = if tm.kind == BUILDING {
            t.annex_of.unwrap_or(target)
        } else {
            target
        };
        let (class, fog_visibility) = (i32::from(tm.class), tm.fog_visibility);
        let (ap, a) = self.object(aimed).ok_or(GenieError::unsupported(
            "an attack task's main building is gone",
        ))?;
        let aimed_at = a.position;
        let aimed_class = i32::from(self.master_of(ap, a.life.master_id)?.class);
        let world_time = self.world_time;
        let unit = &mut self.players[player].objects[idx];
        if let Some(ai) = unit.ai.as_mut() {
            ai.set_action(600);
            ai.set_target(aimed, aimed_class, aimed_at);
        }
        self.take(target, self.releaser(player, idx));
        let unit = &mut self.players[player].objects[idx];
        if let Some(ai) = unit.ai.as_mut() {
            ai.last_retarget_time = world_time;
            ai.notify_work(
                700,
                600,
                Some((target, class)),
                point,
                clears,
                self.streams.get(Stream::Ai),
            )?;
            self.remember_order(player, idx, (700, 600))?;
        }
        self.clear_actions(player, idx)?;
        let master = self.master_of(player, self.players[player].objects[idx].life.master_id)?;
        // the task factory (`FUN_140a97e00` case 7) gives the Attack the task's moving sprite
        let arms =
            crate::combat::attack::arms(master, &self.graphics, task.sprites, task.sprites[0]);
        let standing = arms.idle_sprite.ok_or(crate::turn::NO_STANDING_SPRITE)?;
        let me_at = *self
            .spatial
            .get(self.players[player].objects[idx].id)
            .ok_or(crate::turn::ATTACKER_UNPLACED)?;
        let t_at = *self
            .spatial
            .get_known(target)?
            .ok_or(crate::turn::TARGET_UNPLACED)?;
        let (tx, ty) = (t_at.position[0] as i32, t_at.position[1] as i32);
        let visible = self.spatial.sees_tile(player, tx, ty)
            || (fog_visibility != 0 && self.spatial.explored(player, tx, ty)?);
        let reload = {
            let u = &self.players[player].objects[idx];
            u.reload_stinger + (master.reload - u.reload_powerup)
        };
        let hit_chance = master.hit_chance;
        let swing_sprites = crate::combat::attack::swing_sprites(
            self.players[player].objects[idx].abilities.look.as_ref(),
            master,
            &self.graphics,
        );
        let computer = self.players[player].computer;
        let volleys = self
            .masters
            .get(player)
            .is_some_and(|m| crate::combat::attack::volleys(master, m));
        let mut victim = self.victim(target, player).ok_or(GenieError::unsupported(
            "an attack's target the slice does not hold live",
        ))?;
        let own = {
            let u = &self.players[player].objects[idx];
            let terrain = self.spatial.terrain.as_ref();
            crate::turn::victim_of(
                u,
                master,
                &self.players[player].attributes,
                player,
                terrain,
                self.mode_680,
            )
        };
        let speed = self.players[player].objects[idx]
            .mover
            .as_ref()
            .map_or(0.0, |m| m.stats.speed);
        let e = crate::combat::attack::engagement(
            self.players[player].objects[idx].ai.as_ref(),
            speed,
            master,
            &me_at,
            &t_at,
            world_time,
        )?;
        let unit = &mut self.players[player].objects[idx];
        if unit.sprite.is_none() {
            return Err(GenieError::unsupported(
                "an attack task of a unit whose sprite the slice does not hold",
            )
            .with("object", unit.id)
            .with("master", unit.life.master_id));
        }
        unit.attack_count = 0;
        crate::combat::attack::set_sprite(
            &mut unit.sprite,
            standing,
            self.streams.get(Stream::Sprite),
        );
        let mover = unit.mover.as_mut().ok_or(crate::turn::ATTACKER_UNMOVING)?;
        let crate::combat::attack::Commanded {
            mut attack,
            arrived,
            posted,
            ready,
            failed,
            failed_first,
        } = crate::combat::attack::Attack::command(
            (target, &self.spatial),
            &e,
            arms,
            (visible, tp),
            mover,
        )?;
        if failed_first {
            // readyToAttack's set_state(13) before the start's set_state(4)
            unit.anim_speed = Some(1.0);
        }
        let (id, state) = (unit.id, unit.life.state);
        for (kind, p) in posted {
            if let Some(ai) = unit.ai.as_mut() {
                ai.post(crate::unit_ai::Notify::own(id, kind, p), state)?;
            }
        }
        if failed {
            attack.fail_at_start(
                &mut unit.sprite,
                &mut unit.anim_speed,
                self.streams.get(Stream::Sprite),
            )?;
        }
        if ready {
            let me = crate::movement::Me {
                id: unit.id,
                position: unit.position,
                group: unit.group.id,
                action_state: crate::combat::attack::READY,
            };
            let can_move = unit
                .ai
                .as_ref()
                .is_some_and(|ai| ai.current_order != -1 || ai.stance <= 1);
            let stance = unit.ai.as_ref().map_or(0, |ai| ai.stance);
            let state_position = unit.ai.as_ref().map_or([0.0; 2], |ai| ai.state_position);
            let queue_empty = unit.ai.as_ref().is_some_and(|ai| ai.orders.is_empty());
            let walk = Walk {
                mover: unit.mover.as_mut(),
                me,
                dt: 0.0,
                world: &mut self.spatial,
                plan: None,
            };
            let mut blasted = Vec::new();
            let mut strike = Strike {
                sprite: &mut unit.sprite,
                anim_speed: &mut unit.anim_speed,
                standing: Some(standing),
                volleys,
                attack_timer: &mut unit.attack_timer,
                reload_worldtime: &mut unit.reload_worldtime,
                reload,
                hit_chance,
                victim: &mut victim,
                blasted: &mut blasted,
                sprite_rng: self.streams.get(Stream::Sprite),
                own,
                executed: None,
                fire: None,
                can_move,
                stance,
                state_position,
                posted: Vec::new(),
                queue_empty,
                world_time,
                ai_order: unit.ai.as_ref().map(|ai| ai.current_order),
                ai_type: unit.ai.as_ref().map(|ai| ai.ai_type),
                computer,
                armed: true,
                volley_under_way: unit.next_volley != 0,
                let_go: None,
                forgets: false,
                sends_home: false,
                swing_sprites,
            };
            attack.enter_ready(walk, &mut strike)?;
            if strike.executed.is_some() || strike.fire.is_some() || !strike.posted.is_empty() {
                return Err(GenieError::unsupported(
                    "an attack task's swing at its start",
                ));
            }
        }
        if arrived {
            // the MoveTo's start in state 1 (`0x140aa29e0`, not quiet): slot 0x68 with the
            // standing sprite (slot 0x798) at its speed, `+0x7b8` cleared; the next update's state
            // 4 ends the MoveTo
            if let Some(m) = unit.mover.as_mut() {
                m.shows_standing();
            }
            unit.shows_standing_sprite(
                self.masters.get(player),
                &self.graphics,
                self.streams.get(Stream::Sprite),
            )?;
        }
        attack.target2 = target;
        unit.attack = Some(attack);
        self.take(target, self.releaser(player, idx));
        Ok(())
    }

    /// The hunt's slot 9 (`0x140ac97c0`) on the work command's new target: a new target is
    /// released for the old one (slot 0x68, `FUN_140ac7980`, through `FUN_140a7a230`) and the hunt
    /// starts over.
    pub(super) fn retarget_hunt(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
    ) -> Result<(), GenieError> {
        let quarry = self.quarry(player, target)?;
        let engagement = self.engagement(player, idx, target)?;
        let task = self.players[player].objects[idx]
            .hunt
            .map(|h| h.task)
            .ok_or(GenieError::unsupported(
                "a hunt's retarget without its hunt",
            ))?;
        let arms = self.arms(player, idx, &task)?;
        // FUN_14109d4f0: the target's master is one of the unit's master's drop sites
        let target_master = self.object(target).map(|(_, t)| t.life.master_id);
        let drops_here = self
            .master_of(player, self.players[player].objects[idx].life.master_id)?
            .drop_sites
            .iter()
            .any(|&d| Some(d) == target_master);
        let unit = &mut self.players[player].objects[idx];
        let old = unit.hunt.map(|h| h.target).filter(|&t| t != -1);
        let held = unit.life.amount;
        let (Some(hunt), Some(mover)) = (unit.hunt.as_mut(), unit.mover.as_mut()) else {
            return Err(GenieError::unsupported(
                "a hunter without its hunt or movement state",
            ));
        };
        let unit_player = player;
        if let Some(arrived) = hunt.retarget(
            &quarry,
            unit_player,
            held,
            drops_here,
            &engagement,
            arms,
            mover,
        )? {
            if arrived {
                crate::combat::attack::show_arrived(
                    &arms,
                    &mut unit.sprite,
                    &mut unit.anim_speed,
                    self.streams.get(Stream::Sprite),
                )?;
            }
            // the hunt's slot 0x68 (FUN_140ac7980): FUN_140a7a230 on the new target, which the
            // work command's task finder took below state 3
            let _ = self.retarget_action(player, idx, old.unwrap_or(-1), target, false)?;
        }
        Ok(())
    }

    /// An action's slot 0x68 base (`FUN_140a7a230`) for the unit at `idx`: its target `old` (−1
    /// for none) becomes `new`. The same target changes nothing. A different old target the
    /// world still holds is let go (slot 0x180, [`Turns::release`]) unless the action is a
    /// sub-action (`quiet`, `+0x60`); a new target in state 7 or more is not taken, and the
    /// action is left without one; else it is taken (slot 0x178 with the action's type,
    /// [`Turns::take`]) unless `quiet`. Returns the action's target id (`+0x30`) after it.
    pub(crate) fn retarget_action(
        &mut self,
        player: usize,
        idx: usize,
        old: i32,
        new: i32,
        quiet: bool,
    ) -> Result<i32, GenieError> {
        if old == new {
            return Ok(old);
        }
        if old != -1 && !quiet && self.object(old).is_some() {
            let by = self.releaser(player, idx);
            self.release(old, by)?;
        }
        let Some((_, t)) = self.object(new) else {
            return Ok(-1);
        };
        if 7 <= t.life.state {
            return Ok(-1);
        }
        if !quiet {
            self.take(new, self.releaser(player, idx));
        }
        Ok(new)
    }
}
