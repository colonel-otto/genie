//! The town bell, a kept attack, the garrison task and the work command and its task.

use crate::hunt::{HUNT, HuntTask};
use crate::masters::Master;
use crate::math::rng::Stream;
use crate::order::{
    ATTACK_TASK, BUILDING, GARRISON, MASTER_84, MASTER_1808, VILLAGER, Work, own_order_target,
};
use crate::tasks::{Actor, Diplomacy, enabled};
use crate::turn::{Turns, World};
use crate::unit_ai::{GenieError, HUNT_ACTION, HUNT_ORDER, MOVE_ORDER};

impl Turns {
    /// `FUN_1412067f0` before slot 0x7e8's work command, for a villager or master 1808 and a
    /// target the world holds: the town-bell record ([`Turns::record_town_bell`]); then, with the
    /// garrison filter (3), the flag goes on for a building in state 2 of the player or an ally
    /// whose master takes villagers (`garrison_type` bit 0), with any other filter it goes off.
    /// The UI's flag for the viewing player's building (`+0x1874`) is not held.
    pub(crate) fn town_bell_order(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
        filter: i16,
    ) -> Result<(), GenieError> {
        let unit = &self.players[player].objects[idx];
        let master = self.master_of(player, unit.life.master_id)?;
        if !(master.class == VILLAGER || master.id == MASTER_1808) {
            return Ok(());
        }
        let Some((tp, t)) = self.object(target) else {
            return Ok(());
        };
        let takes = if filter == GARRISON {
            let tm = self.master_of(tp, t.life.master_id)?;
            let world = World {
                players: &self.players,
                drop_sites: &self.drop_sites,
            };
            (tm.kind == BUILDING
                && t.life.state == 2
                && world.allied(player, tp, true)
                && tm.building.is_some_and(|b| b.garrison_type & 1 != 0))
            .then_some(true)
        } else {
            Some(false)
        };
        self.record_town_bell(player, idx)?;
        if let Some(flag) = takes {
            self.players[player].objects[idx].town_bell.flag = flag;
        }
        Ok(())
    }

    /// `FUN_141206a20`: the unit's present work as its town-bell record
    /// ([`crate::town_bell::TownBell::record`]): its top action's target (slot 400), or, for one
    /// in a pasture the world holds, the pasture; without one the target of its unit AI's first
    /// own queued order (`FUN_140c4da50`: issued by the unit, with a target), else none at its own
    /// place. Then a Gather's or Hunt's target class (`+0x68`), its unit AI's current action
    /// (`+0x44`) and the top action's second target (`FUN_140a78430`). The targets count only
    /// where the world holds them.
    pub(crate) fn record_town_bell(&mut self, player: usize, idx: usize) -> Result<(), GenieError> {
        use crate::town_bell::BackToWork;
        let unit = &self.players[player].objects[idx];
        if unit.town_bell.flag {
            return Ok(());
        }
        if unit.unmodelled_action.is_some() {
            return Err(GenieError::unsupported(
                "a town-bell record of an action not held",
            ));
        }
        let held = |id: i32| (0 <= id).then(|| self.object(id)).flatten();
        let mut found = unit.top_target().and_then(held).map(|(_, t)| t);
        if let Some(t) = found
            && t.pasture != -1
        {
            found = held(t.pasture).map(|(_, p)| p);
        }
        if found.is_none() {
            let own = unit
                .ai
                .as_ref()
                .map_or(-1, |ai| own_order_target(ai, unit.id));
            found = held(own).map(|(_, t)| t);
        }
        let (target, point) = found.map_or((-1, [unit.position[0], unit.position[1]]), |t| {
            (t.id, [t.position[0], t.position[1]])
        });
        let target_type = match unit.top_action_kind() {
            Some(HUNT) => unit.hunt.map_or(-1, |h| i32::from(h.target_class)),
            Some(crate::gather::GATHER) => unit.gather.map_or(-1, |g| g.target_type),
            _ => -1,
        };
        let ai = unit.ai.as_ref().ok_or(GenieError::unsupported(
            "a town-bell record of a unit without a unit AI",
        ))?;
        let target2 = unit
            .top_second_target()
            .and_then(held)
            .map_or(-1, |(_, t)| t.id);
        let now = BackToWork {
            target,
            target_type,
            action: ai.current_action,
            target2,
            point,
        };
        self.players[player].objects[idx].town_bell.record(now);
        Ok(())
    }

    /// The work command's check of the unit's top Attack (`FUN_14106ff70`): one on this very
    /// target, the filter none or 9, keeps it when its slot 9 (`FUN_140a841d0`, [`Attack::keeps`])
    /// does, and the unit AI hears of it as of a new one (slot 0x828: the attack order and
    /// action, the target, the retarget time). True when kept.
    ///
    /// [`Attack::keeps`]: crate::attack::Attack::keeps
    pub(super) fn keep_attack(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
        filter: i16,
        point: [f32; 3],
        clears: bool,
    ) -> Result<bool, GenieError> {
        let unit = &self.players[player].objects[idx];
        let top = unit.top_action_kind();
        let attacking = unit
            .attack
            .filter(|a| top == Some(crate::attack::ATTACK) && a.target == target);
        let Some(a) = attacking.filter(|_| matches!(filter, -1 | 9)) else {
            return Ok(false);
        };
        if !a.keeps(target)? {
            return Ok(false);
        }
        let (tp, t) = self.object(target).ok_or(GenieError::unsupported(
            "an attack's target the slice does not hold",
        ))?;
        let class = i32::from(self.master_of(tp, t.life.master_id)?.class);
        let world_time = self.world_time;
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
        Ok(true)
    }

    /// The unit's own task list's finder with filter 3 (slot 2 of its master's `+0x788`,
    /// `FUN_1412784d0`: the garrison's, [`crate::tasks::find_task`]) on `target`: whether it
    /// finds a task, as a gather point on a building asks (`FUN_140ace9c0`).
    pub(crate) fn garrison_task_found(
        &self,
        player: usize,
        idx: usize,
        target: i32,
    ) -> Result<bool, GenieError> {
        let unit = &self.players[player].objects[idx];
        let owner = &self.players[player];
        let master = self.master_of(player, unit.life.master_id)?;
        let actor = Actor {
            id: unit.id,
            master,
            player,
            held: (unit.life.attribute_type, unit.life.amount),
            attribute_95: owner.attributes.get(95).copied().unwrap_or(0.0),
            civ: owner.civ,
        };
        let world = World {
            players: &self.players,
            drop_sites: &self.drop_sites,
        };
        let t = self.task_target(target, 1)?;
        let general = self.task_target(target, 2)?;
        Ok(
            crate::tasks::find_task(master, &actor, Some(&t), Some(&general), None, 3, &world)?
                .is_some(),
        )
    }

    /// The work command's carried-resource test (`+0x12`, `FUN_14106ff70` with `param_8`) on the
    /// task found: a gather or hunt task (action type 5 or 0x6e) of another resource (`+0x20`)
    /// than the unit holds (`+0x308`; the food kinds 0 and 15 to 17 count as one) leaves the
    /// unit holding none of the task's (`FUN_14119d310` with 0); an attack task (7 or 9) of a unit
    /// that is no villager turns its unit AI's retargeting off (`+0x1d6`, `FUN_140c40ef0` with
    /// the test's 1).
    pub(super) fn carried_resource_test(
        &mut self,
        player: usize,
        idx: usize,
        task: &crate::masters::Task,
    ) -> Result<(), GenieError> {
        let food = |k: i16| k == 0 || (15..=17).contains(&k);
        let class = self
            .master_of(player, self.players[player].objects[idx].life.master_id)?
            .class;
        let unit = &mut self.players[player].objects[idx];
        if matches!(task.action_type, crate::gather::GATHER | HUNT) {
            let (kind, held) = (task.attribute_type, unit.life.attribute_type);
            if held != kind && !(food(kind) && food(held)) {
                unit.life.attribute_type = kind;
                unit.life.amount = 0.0;
            }
        }
        if matches!(task.action_type, ATTACK_TASK | crate::attack::ATTACK)
            && class != VILLAGER
            && let Some(ai) = unit.ai.as_mut()
        {
            ai.retarget_disabled = 1;
        }
        Ok(())
    }

    /// The work command (slot 37, `FUN_14106ff70`) of a unit on a target object: the master and
    /// task of [`crate::masters::Masters::work_master`] (the master changes, slot 0x90), the task's
    /// requirement, then the task's action from the action list's factory (slot 0x10), the unit
    /// AI's notify (slot 0x828) and the install (slot 0x808).
    pub(super) fn work_command(
        &mut self,
        player: usize,
        idx: usize,
        order: &Work,
    ) -> Result<(), GenieError> {
        let Work {
            target,
            point,
            carry_test,
            filter,
            notified,
        } = *order;
        let unit = &self.players[player].objects[idx];
        if unit.id == target {
            return Ok(());
        }
        // FUN_140c64e50: a unit of a type below 70 always clears its AI's notifies
        let clears = notified || self.master_of(player, unit.life.master_id)?.kind < 70;
        // a work command on a point (no target) finds no task
        let found = if target == -1 {
            None
        } else {
            let masters = self
                .masters
                .get(player)
                .ok_or(GenieError::unsupported("a work command: no master table"))?;
            let owner = &self.players[player];
            let actor = Actor {
                id: unit.id,
                master: self.master_of(player, unit.life.master_id)?,
                player,
                held: (unit.life.attribute_type, unit.life.amount),
                attribute_95: owner.attributes.get(95).copied().unwrap_or(0.0),
                civ: owner.civ,
            };
            let world = World {
                players: &self.players,
                drop_sites: &self.drop_sites,
            };
            let t = self.task_target(target, 1)?;
            let general = self.task_target(target, 2)?;
            masters
                .work_master(&actor, &t, &general, filter, &world)?
                .map(|(with, task)| (with, *task))
        };
        let Some((with, task)) = found else {
            if filter != -1 {
                return Ok(());
            }
            // LAB_1410706a8 without a task: the top Attack on the target keeps it; else a unit
            // whose master moves (slot 0xe8, its speed `+0x740`) is told of a move (slot 0x828)
            // and moves there (slot 0x138), which is not modelled; one that does not, nothing
            if self.keep_attack(player, idx, target, filter, point, clears)? {
                return Ok(());
            }
            return self.work_move(player, idx, target, point, (clears, notified));
        };
        // a new master's task is another than the one the unit's action holds
        let same_master = with == self.players[player].objects[idx].life.master_id;
        if !same_master {
            self.change_master(player, idx, with)?;
        }
        if !enabled(&self.players[player].attributes, task.enablement) {
            return Ok(());
        }
        // LAB_1410706a8: an action on the task found takes the new target itself (its slot 9)
        let unit = &self.players[player].objects[idx];
        let top = unit.top_action_kind();
        let kept = match unit.hunt {
            Some(h) if task.action_type == HUNT && h.task.id == task.id => {
                let t = self.task_target(target, 0)?;
                if t.master.id == MASTER_84 && t.player == player {
                    return Err(GenieError::unsupported(
                        "a hunt sent to its player's master 84",
                    ));
                }
                let class = i32::from(t.master.class);
                self.retarget_hunt(player, idx, target)?;
                let unit = &mut self.players[player].objects[idx];
                if let Some(ai) = unit.ai.as_mut() {
                    ai.notify_work(
                        HUNT_ORDER,
                        HUNT_ACTION,
                        Some((target, class)),
                        point,
                        clears,
                        self.streams.get(Stream::Ai),
                    )?;
                    self.remember_order(player, idx, (HUNT_ORDER, HUNT_ACTION))?;
                }
                true
            }
            _ if top == Some(task.action_type) && task.action_type == crate::build::BUILD => {
                if !same_master {
                    return Err(GenieError::unsupported(
                        "a work command on the Build task of the unit's former master",
                    ));
                }
                self.retake_build(player, idx, target, point, clears)?;
                true
            }
            _ if top == Some(task.action_type) && task.action_type == crate::repair::REPAIR => {
                self.retake_repair(player, idx, target, point, clears)?;
                true
            }
            _ if top == Some(crate::gather::GATHER)
                && task.action_type == crate::gather::GATHER
                && unit.gather.is_some_and(|g| g.task.id == task.id) =>
            {
                let t = self.task_target(target, 0)?;
                if t.master.id == MASTER_84 && t.player == player {
                    return Err(GenieError::unsupported(
                        "a gather sent to its player's master 84",
                    ));
                }
                let class = i32::from(t.master.class);
                let took = self.retarget_gather(player, idx, target)?;
                if took {
                    let unit = &mut self.players[player].objects[idx];
                    if let Some(ai) = unit.ai.as_mut() {
                        ai.notify_work(
                            crate::ai_kinds::ORDER_GATHER,
                            crate::unit_ai::GATHER_ACTION,
                            Some((target, class)),
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
                }
                took
            }
            // the top action's slot 0x48 (FUN_140a977a0) on the same task: the base's
            // (FUN_140a7bf40: the MoveTo's, the Enter's, the missile's, the Fly's, the Make's, the
            // wait for a foundation's) answers 0, and the work goes on to the factory
            _ if top == Some(task.action_type) && top != Some(HUNT) => match top {
                Some(
                    crate::move_to::MOVE_TO
                    | crate::garrison::ENTER
                    | crate::missile::MISSILE
                    | crate::fly::FLY
                    | crate::make::MAKE
                    | crate::wait_solidify::WAIT_SOLIDIFY,
                ) => false,
                _ => {
                    return Err(GenieError::unsupported(
                        "a work command on the unit's current task",
                    )
                    .with("action", format_args!("{top:?}"))
                    .with("task", task.id)
                    .with("target", target));
                }
            },
            _ => false,
        };
        if carry_test {
            self.carried_resource_test(player, idx, &task)?;
        }
        if self.keep_attack(player, idx, target, filter, point, clears)? {
            return Ok(());
        }
        // the top action of type 0x7e would hear the target through its slot 0x48: the slice
        // holds no action of that type. An Artifact on top ends a command of an Enter task here.
        if top == Some(crate::artifact::ARTIFACT_TYPE) && task.action_type == crate::garrison::ENTER
        {
            return Ok(());
        }
        if kept {
            return Ok(());
        }
        match task.action_type {
            crate::garrison::ENTER => self.start_enter(player, idx, target, &task, point, clears),
            ATTACK_TASK => self.start_attack(player, idx, target, &task, point, clears),
            HUNT => self.start_hunt(
                player,
                idx,
                target,
                HuntTask {
                    id: task.id,
                    range: task.work_range,
                    resource: task.attribute_type,
                    sprites: task.sprites,
                    work_value: task.work_val1,
                    productivity: task.attribute_type2,
                    kind4: task.attribute_type4,
                },
                point,
                clears,
                true,
            ),
            crate::build::BUILD => self.start_build(player, idx, target, &task, point, clears),
            crate::repair::REPAIR => self.start_repair(player, idx, target, &task, point, clears),
            // the factory makes no action of a mule cart's task: the move without one
            crate::build::CART_TASK => {
                self.work_move(player, idx, target, point, (clears, notified))
            }
            crate::gather::GATHER => {
                self.start_gather(player, idx, target, &task, Some((point, clears)))
            }
            _ => Err(GenieError::unsupported(
                "the action factory for a task the slice does not model",
            )),
        }
    }

    /// The work command without an action (`FUN_14106ff70`: no task, or one the action list's
    /// factory makes no action of, the mule cart's 0x99): a unit whose master moves (slot 0xe8,
    /// its speed `+0x740`) is told of a move (slot 0x828, its notifies cleared with `clears`) and,
    /// in state 2 or of class 51, moves to the target or the point (slot 0x138 with the unit's
    /// `+0xa14`, the work's `notified`); one that does not move does nothing.
    pub(super) fn work_move(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
        point: [f32; 3],
        (clears, notified): (bool, bool),
    ) -> Result<(), GenieError> {
        let unit = &self.players[player].objects[idx];
        let master = self.master_of(player, unit.life.master_id)?;
        if !master.moving.is_some_and(|m| 0.0 < m.speed) {
            return Ok(());
        }
        // slot 0x828 with 1 (the move's order and action) for a unit in state 2, -1 for
        // another; then the move to the target (slot 0x138) for a unit in state 2 or of
        // class 51
        let (alive, class) = (unit.life.state == 2, master.class);
        let named = if target == -1 {
            None
        } else {
            let (tp, t) = self.object(target).ok_or(GenieError::unsupported(
                "a work command's target the slice does not hold",
            ))?;
            Some((
                target,
                i32::from(self.master_of(tp, t.life.master_id)?.class),
            ))
        };
        let (order, action) = if alive {
            (MOVE_ORDER, crate::unit_ai::MOVE_ACTION)
        } else {
            (-1, -1)
        };
        let unit = &mut self.players[player].objects[idx];
        if let Some(ai) = unit.ai.as_mut() {
            ai.notify_work(
                order,
                action,
                named,
                point,
                clears,
                self.streams.get(Stream::Ai),
            )?;
            self.remember_order(player, idx, (order, action))?;
        }
        if alive || class == 51 {
            if target == -1 {
                self.move_point_order_with(player, idx, point, notified)?;
            } else {
                let _ = self.move_to_object_with(player, idx, target, point, notified)?;
            }
        }
        Ok(())
    }

    /// The unit's slot 0x820 (`FUN_14106c6a0`) on `target` (its place the point): the task its
    /// master's list gives for the target with `filter` (the list's slot 0x10, [`find_task`] in
    /// mode 0); without one, a master with a task swap group (`+0x7b8` not 0) tries each of its
    /// player's masters by id of the same class, type and group, its own too, with any filter;
    /// the first with a task gives it, and with `change` the unit takes that master (slot 0x90,
    /// [`Turns::change_master`], or [`Turns::transform_building`] for a building) though it be its
    /// own. None: no task. The terrain under the point, which only a task for a terrain reads, is
    /// not passed.
    ///
    /// [`find_task`]: crate::tasks::find_task
    pub(crate) fn find_work_task(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
        filter: i16,
        change: bool,
    ) -> Result<Option<crate::masters::Task>, GenieError> {
        let unit = &self.players[player].objects[idx];
        let own = self.master_of(player, unit.life.master_id)?;
        let owner = &self.players[player];
        let actor = Actor {
            id: unit.id,
            master: own,
            player,
            held: (unit.life.attribute_type, unit.life.amount),
            attribute_95: owner.attributes.get(95).copied().unwrap_or(0.0),
            civ: owner.civ,
        };
        let world = World {
            players: &self.players,
            drop_sites: &self.drop_sites,
        };
        let t = self.task_target(target, 1)?;
        let general = self.task_target(target, 2)?;
        let find = |list: &Master, filter: i16| {
            crate::tasks::find_task(list, &actor, Some(&t), Some(&general), None, filter, &world)
                .map(Option::<&crate::masters::Task>::copied)
        };
        if let Some(task) = find(own, filter)? {
            return Ok(Some(task));
        }
        if own.task_by_group == 0 {
            return Ok(None);
        }
        let mut found = None;
        // the master table exists: master_of found the unit's own in it
        for candidate in self.masters[player].iter() {
            if candidate.class != own.class
                || candidate.kind != own.kind
                || candidate.task_by_group != own.task_by_group
            {
                continue;
            }
            if let Some(task) = find(candidate, -1)? {
                found = Some((candidate.id, task));
                break;
            }
        }
        let Some((with, task)) = found else {
            return Ok(None);
        };
        if change {
            if self.master_of(player, with)?.kind == BUILDING {
                self.transform_building(player, idx, with)?;
            } else {
                self.change_master(player, idx, with)?;
            }
        }
        Ok(Some(task))
    }
}
