//! A gather's retarget and take, the quarry, a gather's and a hunt's start.

use crate::combat::attack::{Arms, Engagement};
use crate::hunt::{HuntAction, HuntTask, Quarry};
use crate::math::rng::Stream;
use crate::order::{BUILDING, GATHERED_BUILDING, MASTER_1893, MASTER_1897, PACK_ACTION};
use crate::tasks::Diplomacy;
use crate::turn::{Turns, World};
use crate::unit_ai::{GenieError, HUNT_ACTION, HUNT_ORDER};

impl Turns {
    /// The Gather's slot 0x48 (`0x140a903a0`) on the work command's new target
    /// ([`crate::gather::GatherAction::retarget`]): false when it does not take it, for a building
    /// (type 80 without object flag 0x20) other than a farm, so the work command makes a new
    /// action. The targets that changed are let go (slot 0x180) and taken (slot 0x178). Not
    /// modelled: a building packing or unpacking (its top action 0x7d), a foundation or master
    /// 1893 or 1897 of the unit's player (slots 0x808 and 0x7e8: its build).
    pub(super) fn retarget_gather(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
    ) -> Result<bool, GenieError> {
        let (tp, t) = self.object(target).ok_or(GenieError::unsupported(
            "a gather's target the slice does not hold",
        ))?;
        let tm = self.master_of(tp, t.life.master_id)?;
        if tm.kind == BUILDING {
            if t.top_action_kind() == Some(PACK_ACTION) || t.unmodelled_action.is_some() {
                return Err(GenieError::unsupported(
                    "a gather retargeted to a building with an action (packing)",
                ));
            }
            if tm.object_flags & GATHERED_BUILDING == 0 {
                let (mp, main) = match t.annex_of {
                    Some(m) => self.object(m).ok_or(GenieError::unsupported(
                        "an annex whose main building is gone",
                    ))?,
                    None => (tp, t),
                };
                let main_master = self.master_of(mp, main.life.master_id)?.id;
                if (main.life.state < 2 || matches!(main_master, MASTER_1893 | MASTER_1897))
                    && mp == player
                {
                    return Err(GenieError::unsupported(
                        "a gather retargeted to its player's foundation (its build)",
                    ));
                }
                if tm.class != crate::gather::FARM_CLASS {
                    return Ok(false);
                }
            }
        }
        let (t_at, t_radius) = (t.position, tm.radius);
        let unit = &self.players[player].objects[idx];
        let me = self.master_of(player, unit.life.master_id)?;
        let resource = crate::gather::Resource {
            id: target,
            position: t_at,
            class: tm.class,
            master: tm.id,
            player: tp,
            workers: t.workers,
            drop_site: self.drops_at_main(player, idx, target)?,
            gap: crate::movement::footprint_gap(unit.position, me.radius, t_at, t_radius),
            radius: t_radius,
        };
        let computer = self.players[player].computer;
        let farm_stops = self.settings.walkable_farms;
        let by = self.releaser(player, idx);
        // a stop farm's time per stop: the gatherer's work rate (its master's plus its powerup),
        // capacity and the task's productivity attribute (none: 1.0)
        let productivity = unit.gather.map_or(-1, |g| g.task.productivity);
        let rate = crate::gather::StopRate {
            work_rate: unit.work_rate_powerup + me.work_rate,
            capacity: f32::from(me.capacity),
            productivity: crate::gather::productivity_of(
                productivity,
                &self.players[player].attributes,
            ),
        };
        let unit = &mut self.players[player].objects[idx];
        let held = unit.life.amount;
        let (Some(gather), Some(mover)) = (unit.gather.as_mut(), unit.mover.as_mut()) else {
            return Err(GenieError::unsupported(
                "a gatherer without its gather or movement state",
            ));
        };
        let (old_target, old_second) = (gather.target, gather.carry_to);
        let rng = self.streams.get(Stream::Action);
        let took = gather.retarget(
            &resource,
            player,
            computer,
            held,
            (farm_stops, rate),
            mover,
            rng,
        )?;
        let (new_target, new_second) = (gather.target, gather.carry_to);
        // a farm another farmer works: taken (slot 0x68) and let go again (set_state 11)
        let waits = gather.state == crate::gather::WALK_BACK && new_target == -1;
        for (old, new) in [(old_target, new_target), (old_second, new_second)] {
            if old != new {
                if old != -1 {
                    self.release(old, by)?;
                }
                if new != -1 {
                    self.take(new, by);
                }
            }
        }
        if waits && old_target != target {
            self.take(target, by);
            self.release(target, by)?;
        }
        Ok(took)
    }

    /// The Gather's slot 0x40 (`0x140a8b740`, [`crate::gather::GatherAction::take_move`]) on a
    /// move of the gatherer at `idx` to `point` toward the object `target`: always taken; the
    /// targets that changed are let go and taken.
    pub(crate) fn gather_take_move(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
        point: [f32; 3],
    ) -> Result<bool, GenieError> {
        let (tp, o) = self.object(target).ok_or(GenieError::unsupported(
            "a move to an object the slice does not hold",
        ))?;
        let tm = self.master_of(tp, o.life.master_id)?;
        let (at, radius) = (o.position, tm.radius);
        let unit = &self.players[player].objects[idx];
        let me = self.master_of(player, unit.life.master_id)?;
        let moved_to = Some(crate::gather::MoveTarget {
            id: target,
            player: tp,
            drop_site: self.drops_at_main(player, idx, target)?,
            gap: crate::movement::footprint_gap(unit.position, me.radius, at, radius),
        });
        let by = self.releaser(player, idx);
        let unit = &mut self.players[player].objects[idx];
        let held = unit.life.amount;
        let (Some(gather), Some(mover)) = (unit.gather.as_mut(), unit.mover.as_mut()) else {
            return Err(GenieError::unsupported(
                "a gatherer without its gather or movement state",
            ));
        };
        let (old_target, old_second) = (gather.target, gather.carry_to);
        gather.take_move(moved_to, point, player, held, mover)?;
        let (new_target, new_second) = (gather.target, gather.carry_to);
        for (old, new) in [(old_target, new_target), (old_second, new_second)] {
            if old != new {
                if old != -1 {
                    self.release(old, by)?;
                }
                if new != -1 {
                    self.take(new, by);
                }
            }
        }
        Ok(true)
    }

    /// `FUN_14109d4f0` of the unit at `idx` on `target`, or on a building annex's main building
    /// (`+0xbb8`): the unit's master has that master among its drop sites.
    pub(super) fn drops_at_main(
        &self,
        player: usize,
        idx: usize,
        target: i32,
    ) -> Result<bool, GenieError> {
        let (tp, t) = self.object(target).ok_or(GenieError::unsupported(
            "a hunt's target the slice does not hold",
        ))?;
        let tm = self.master_of(tp, t.life.master_id)?;
        let main = if tm.kind == BUILDING {
            t.annex_of.unwrap_or(target)
        } else {
            target
        };
        let main_master = match self.object(main) {
            Some((mp, m)) => self.master_of(mp, m.life.master_id)?.id,
            None => tm.id,
        };
        let unit = &self.players[player].objects[idx];
        Ok(self
            .master_of(player, unit.life.master_id)?
            .drop_sites
            .contains(&main_master))
    }

    /// The Hunt's slot 0x40 from a move toward the object `target` ([`HuntAction::take_move`])
    /// with `point`: true when it takes it. The target it had goes (slot 0x68 with none:
    /// [`Turns::release`]), and a drop site it carries its load to becomes its second target
    /// (`FUN_140a7a190`: [`Turns::take`]).
    ///
    /// [`HuntAction::take_move`]: crate::hunt::HuntAction::take_move
    pub(crate) fn hunt_take_move(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
        point: [f32; 3],
    ) -> Result<bool, GenieError> {
        let mut q = self.quarry(player, target)?;
        q.drop_site = self.drops_at_main(player, idx, target)?;
        let quarry = Some(q);
        let gap = self.engagement(player, idx, target)?.gap;
        let by = self.releaser(player, idx);
        let unit = &mut self.players[player].objects[idx];
        let (held, my_pasture) = (unit.life.amount, unit.my_pasture);
        let (Some(hunt), Some(mover)) = (unit.hunt.as_mut(), unit.mover.as_mut()) else {
            return Err(GenieError::unsupported(
                "a hunter without its hunt or movement state",
            ));
        };
        let (old_target, old_second) = (hunt.target, hunt.carry_to);
        if !hunt.take_move(
            quarry.as_ref(),
            point,
            player,
            held,
            quarry.as_ref().is_some_and(|q| q.drop_site),
            my_pasture,
            gap,
            mover,
        )? {
            return Ok(false);
        }
        let new_second = hunt.carry_to;
        if old_target != -1 && hunt.target == -1 {
            self.release(old_target, by)?;
        }
        if new_second != old_second && new_second != -1 {
            if old_second != -1 {
                self.release(old_second, by)?;
            }
            self.take(new_second, by);
        }
        Ok(true)
    }

    /// What a hunt reads of its target.
    pub(super) fn quarry(&self, player: usize, target: i32) -> Result<Quarry, GenieError> {
        let (tp, t) = self.object(target).ok_or(GenieError::unsupported(
            "a hunt's target the slice does not hold",
        ))?;
        let t_master = self.master_of(tp, t.life.master_id)?;
        let world = World {
            players: &self.players,
            drop_sites: &self.drop_sites,
        };
        Ok(Quarry {
            id: target,
            position: t.position,
            class: t_master.class,
            kind: t_master.kind,
            state: t.life.state,
            player: tp,
            amount: t.life.amount,
            allied: world.allied(player, tp, true),
            top_action: t.top_action_kind(),
            drop_site: false,
        })
    }

    /// The work command's Gather (its task of action type 5, the base factory `FUN_140a97e00`
    /// case 5 with a target): the constructor ([`crate::gather::GatherAction::new`]) takes its
    /// target (slot 0x178); the unit AI's notify (slot 0x828: the gather order and action, the
    /// target with its class, the work's point; `notify`'s point and clearing, None for the gather
    /// command's, which has none); the install (slot 0x808) clears the action list and starts the
    /// Gather ([`crate::gather::GatherAction::start`]). A load to its own drop site takes the
    /// target as the second target, then lets it go as the first.
    pub(super) fn start_gather(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
        task: &crate::masters::Task,
        notify: Option<([f32; 3], bool)>,
    ) -> Result<(), GenieError> {
        let (tp, t) = self.object(target).ok_or(GenieError::unsupported(
            "a gather's target the slice does not hold",
        ))?;
        let tm = self.master_of(tp, t.life.master_id)?;
        let (t_at, t_radius) = (t.position, tm.radius);
        let unit = &self.players[player].objects[idx];
        let me = self.master_of(player, unit.life.master_id)?;
        let resource = crate::gather::Resource {
            id: target,
            position: t_at,
            class: tm.class,
            master: tm.id,
            player: tp,
            workers: t.workers,
            drop_site: self.drops_at_main(player, idx, target)?,
            gap: crate::movement::footprint_gap(unit.position, me.radius, t_at, t_radius),
            radius: t_radius,
        };
        let task = HuntTask {
            id: task.id,
            range: task.work_range,
            resource: task.attribute_type,
            sprites: task.sprites,
            work_value: task.work_val1,
            productivity: task.attribute_type2,
            kind4: task.attribute_type4,
        };
        // the constructor's time per stop: the gatherer's work rate (its master's plus its
        // powerup, slot 0x6e8), capacity and productivity attribute (none: 1.0)
        let rate = crate::gather::StopRate {
            work_rate: unit.work_rate_powerup + me.work_rate,
            capacity: f32::from(me.capacity),
            productivity: crate::gather::productivity_of(
                task.productivity,
                &self.players[player].attributes,
            ),
        };
        let mut gather =
            crate::gather::GatherAction::new(task, &resource, self.settings.walkable_farms, rate)?;
        self.take(target, self.releaser(player, idx));
        let unit = &mut self.players[player].objects[idx];
        if let (Some(ai), Some((point, clears))) = (unit.ai.as_mut(), notify) {
            ai.notify_work(
                crate::ai_kinds::ORDER_GATHER,
                crate::unit_ai::GATHER_ACTION,
                Some((target, i32::from(resource.class))),
                point,
                clears,
                self.streams.get(Stream::Ai),
            )?;
            self.remember_order(
                player,
                idx,
                (crate::ai_kinds::ORDER_GATHER, crate::unit_ai::GATHER_ACTION),
            )?;
        }
        self.clear_actions(player, idx)?;
        let computer = self.players[player].computer;
        let carry_rule = self.settings.carry_rule();
        let unit = &mut self.players[player].objects[idx];
        let mover = unit
            .mover
            .as_mut()
            .ok_or(GenieError::unsupported("a gatherer without movement state"))?;
        let mut held = (unit.life.attribute_type, unit.life.amount);
        let rng = self.streams.get(Stream::Action);
        gather.start(
            &resource, player, computer, carry_rule, &mut held, mover, rng,
        )?;
        (unit.life.attribute_type, unit.life.amount) = held;
        let drops = gather.target == -1 && gather.carry_to == target;
        // a farm another farmer works: the start let it go (set_state 11, then slot 0x68)
        let waits = gather.target == -1 && gather.carry_to != target;
        unit.gather = Some(gather);
        if drops {
            self.take(target, self.releaser(player, idx));
        }
        if drops || waits {
            let by = self.releaser(player, idx);
            self.release(target, by)?;
        }
        Ok(())
    }

    /// [`crate::combat::attack::arms`] of the unit at `idx` for its hunt's Attack (Hunt's `set_state` 12:
    /// the `MoveTo` shows the hunt's walking sprite, [`HuntTask::walk_sprite_id`]).
    pub(super) fn arms(
        &self,
        player: usize,
        idx: usize,
        task: &HuntTask,
    ) -> Result<Arms, GenieError> {
        let unit = &self.players[player].objects[idx];
        let master = self.master_of(player, unit.life.master_id)?;
        let moving = task.walk_sprite_id(unit.life.attribute_type, unit.life.amount);
        Ok(crate::combat::attack::arms(
            master,
            &self.graphics,
            task.sprites,
            moving,
        ))
    }

    /// [`crate::combat::attack::engagement`] of the unit at `idx` with `target`.
    pub(super) fn engagement(
        &self,
        player: usize,
        idx: usize,
        target: i32,
    ) -> Result<Engagement, GenieError> {
        let unit = &self.players[player].objects[idx];
        let master = self.master_of(player, unit.life.master_id)?;
        let me = self
            .spatial
            .get(unit.id)
            .ok_or(crate::turn::ATTACKER_UNPLACED)?;
        let them = self
            .spatial
            .get_known(target)?
            .ok_or(crate::turn::TARGET_UNPLACED)?;
        let speed = unit.mover.as_ref().map_or(0.0, |m| m.stats.speed);
        crate::combat::attack::engagement(
            unit.ai.as_ref(),
            speed,
            master,
            me,
            them,
            self.world_time,
        )
    }

    /// The Hunt from the factory (`FUN_140aca4b0` case 0x6e): the action takes its target (slot
    /// 0x178); the work command's unit AI notify (`FUN_14106e4e0`, `notify`) sets the hunt order;
    /// the install (`FUN_14106e990`) clears the action list and starts the hunt.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn start_hunt(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
        task: HuntTask,
        point: [f32; 3],
        clears: bool,
        notify: bool,
    ) -> Result<(), GenieError> {
        let mut quarry = self.quarry(player, target)?;
        quarry.drop_site = self.drops_at_main(player, idx, target)?;
        self.take(target, self.releaser(player, idx));
        let unit = &mut self.players[player].objects[idx];
        if let Some(ai) = unit.ai.as_mut().filter(|_| notify) {
            ai.notify_work(
                HUNT_ORDER,
                HUNT_ACTION,
                Some((target, i32::from(quarry.class))),
                point,
                clears,
                self.streams.get(Stream::Ai),
            )?;
            self.remember_order(player, idx, (HUNT_ORDER, HUNT_ACTION))?;
        }
        self.clear_actions(player, idx)?;
        let engagement = self.engagement(player, idx, target)?;
        let arms = self.arms(player, idx, &task)?;
        let computer = self.players[player].computer;
        let carry_rule = self.settings.carry_rule();
        let unit = &mut self.players[player].objects[idx];
        let mover = unit.mover.as_mut().ok_or(crate::turn::HUNTER_UNMOVING)?;
        let mut held = (unit.life.attribute_type, unit.life.amount);
        let (hunt, arrived) = HuntAction::start(
            task,
            &quarry,
            player,
            computer,
            carry_rule,
            &mut held,
            &engagement,
            arms,
            mover,
        )?;
        (unit.life.attribute_type, unit.life.amount) = held;
        // an Attack whose MoveTo started arrived: the unit shows the idle sprite (slot 0x68) and
        // its `+0x7b8` clears (`Attack::arrived_sprite`)
        if arrived {
            let g = arms.idle_sprite.ok_or(crate::turn::NO_STANDING_SPRITE)?;
            crate::combat::attack::set_sprite(
                &mut unit.sprite,
                g,
                self.streams.get(Stream::Sprite),
            );
            if let Some(a) = unit.anim_speed.as_mut() {
                *a = 0.0;
            }
        }
        let drops = hunt.target == -1 && hunt.carry_to == target;
        unit.hunt = Some(hunt);
        // a load to its own drop site: the target taken as the second (FUN_140a7a190), then let
        // go as the first (slot 0x68 with none)
        if drops {
            self.take(target, self.releaser(player, idx));
            let by = self.releaser(player, idx);
            self.release(target, by)?;
        }
        Ok(())
    }
}
