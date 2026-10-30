//! A kept attack, the garrison task and the work command and its task.

use crate::GenieError;
use crate::gathering::gather::GATHER;
use crate::gathering::hunt::{HUNT, HuntTask};
use crate::math::rng::Stream;
use crate::objects::masters::Master;
use crate::objects::masters::action_type;
use crate::objects::masters::class;
use crate::objects::masters::ids::MARKET;
use crate::objects::masters::master_type::BUILDING;
use crate::objects::masters::master_type::COMBAT;
use crate::orders::gather::HuntOrder;
use crate::orders::{ATTACK_TASK, Work};
use crate::unit_ai::ActionKind;
use crate::unit_ai::OrderKind;
use crate::unit_ai::task_finder::world::TaskWorld;
use crate::unit_ai::task_finder::{Actor, enabled};
use crate::world::World;
use crate::world::slots::Held;
use crate::world::slots::PlayerSlots;

impl World {
    /// The work command's check of the unit's top Attack (`FUN_14106ff70`): one on this very
    /// target, the filter none or 9, keeps it when its slot 9 (`FUN_140a841d0`, [`Attack::keeps`])
    /// does, and the unit AI hears of it as of a new one (slot 0x828: the attack order and
    /// action, the target, the retarget time). True when kept.
    ///
    /// [`Attack::keeps`]: crate::combat::attack::Attack::keeps
    pub(super) fn keep_attack(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
        filter: i16,
        point: [f32; 3],
        clears: bool,
    ) -> Result<bool, GenieError> {
        let unit = &self.players.live(player, idx)?;
        let top = unit.top_action_kind();
        let attacking = unit
            .actions
            .attack()
            .filter(|a| top == Some(crate::combat::attack::ATTACK) && a.target == target);
        let Some(a) = attacking.filter(|_| matches!(filter, -1 | 9)) else {
            return Ok(false);
        };
        if !a.keeps(target)? {
            return Ok(false);
        }
        let (tp, t) = self.object(target).ok_or(GenieError::unsupported(
            "an attack's target the slice does not hold",
        ))?;
        let class = i32::from(t.master(&self.masters, tp)?.class);
        let world_time = self.world_time;
        let unit = self.players.live_mut(player, idx)?;
        if let Some(ai) = unit.ai.as_mut() {
            ai.last_retarget_time = world_time;
            ai.notify_work(
                Some(OrderKind::Attack),
                Some(ActionKind::Attack),
                Some((target, class)),
                point,
                clears,
                self.streams.get(Stream::Ai),
            )?;
            self.remember_order(
                player,
                idx,
                (Some(OrderKind::Attack), Some(ActionKind::Attack)),
            )?;
        }
        Ok(true)
    }

    /// The unit at `idx` of player `player` as the task finder's actor.
    ///
    /// # Errors
    ///
    /// A unit or a master the world does not hold.
    fn garrison_actor(&self, player: usize, idx: usize) -> Result<Actor<'_>, GenieError> {
        let owner = self.players.player(player)?;
        let unit = owner.live(idx)?;
        Ok(Actor {
            id: unit.id,
            master: unit.master(&self.masters, player)?,
            player,
            held: (unit.life.attribute_type, unit.life.amount),
            attribute_95: owner.attributes.get(95).copied().unwrap_or(0.0),
            civ: owner.civ,
        })
    }

    /// The unit's own task list's finder with filter 3 (slot 2 of its master's `+0x788`,
    /// `FUN_1412784d0`: the garrison's, [`crate::unit_ai::task_finder::find_task`]) on `target` at
    /// the gather `point`: whether it finds a task, as a gather point on a building asks
    /// (`FUN_140ace9c0`).
    pub(crate) fn garrison_task_found(
        &self,
        player: usize,
        idx: usize,
        target: i32,
        point: [f32; 3],
    ) -> Result<bool, GenieError> {
        let actor = self.garrison_actor(player, idx)?;
        let master = actor.master;
        let world = TaskWorld {
            players: &self.players,
            drop_sites: &self.drop_sites,
        };
        let t = self.task_target(target, 1)?;
        let general = self.task_target(target, 2)?;
        Ok(crate::unit_ai::task_finder::find_task(
            master,
            &actor,
            Some(&t),
            Some(&general),
            self.task_terrain(point),
            3,
            &world,
        )?
        .is_some())
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
        task: &crate::objects::masters::Task,
    ) -> Result<(), GenieError> {
        let food = |k: i16| k == 0 || (15..=17).contains(&k);
        let class = self
            .players
            .live(player, idx)?
            .master(&self.masters, player)?
            .class;
        let unit = self.players.live_mut(player, idx)?;
        if matches!(task.action_type, GATHER | HUNT) {
            let (kind, held) = (task.attribute_type, unit.life.attribute_type);
            if held != kind && !(food(kind) && food(held)) {
                unit.life.attribute_type = kind;
                unit.life.amount = 0.0;
            }
        }
        if matches!(
            task.action_type,
            ATTACK_TASK | crate::combat::attack::ATTACK
        ) && class != class::VILLAGER
            && let Some(ai) = unit.ai.as_mut()
        {
            ai.retarget_disabled = 1;
        }
        Ok(())
    }

    /// The work command (slot 37, `FUN_14106ff70`) of a unit on a target object: the master and
    /// task of [`crate::objects::masters::Masters::work_master`] (the master changes, slot 0x90), the task's
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
        let unit = &self.players.live(player, idx)?;
        if unit.id == target {
            return Ok(());
        }
        // FUN_140c64e50: a unit of a type below 70 always clears its AI's notifies
        let clears = notified || unit.master(&self.masters, player)?.kind < COMBAT;
        let found = self.work_task(player, idx, (target, point), filter)?;
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
        let same_master = with == self.players.live(player, idx)?.life.master_id;
        if !same_master {
            self.change_master(player, idx, with)?;
        }
        if !enabled(&self.players.player(player)?.attributes, task.enablement) {
            return Ok(());
        }
        // LAB_1410706a8: an action on the task found takes the new target itself (its slot 9)
        let top = self.players.live(player, idx)?.top_action_kind();
        let kept = self.retake_task(player, idx, (target, point, clears), &task, same_master)?;
        if carry_test {
            self.carried_resource_test(player, idx, &task)?;
        }
        if self.keep_attack(player, idx, target, filter, point, clears)? {
            return Ok(());
        }
        // the top action of type 0x7e would hear the target through its slot 0x48: the slice
        // holds no action of that type. An Artifact on top ends a command of an Enter task here.
        if top == Some(crate::ownership::artifact::ARTIFACT_TYPE)
            && task.action_type == crate::garrison::ENTER
        {
            return Ok(());
        }
        if kept {
            return Ok(());
        }
        self.start_task(player, idx, (target, point), &task, (clears, notified))
    }

    /// The work command's master and task for the unit at (`player`, `idx`) on `target` (−1 for
    /// none) at `point` ([`crate::objects::masters::Masters::work_master`] under the task
    /// `filter`, the terrain under the point for a task for a terrain): the master with the task
    /// found.
    pub(super) fn work_task(
        &self,
        player: usize,
        idx: usize,
        (target, point): (i32, [f32; 3]),
        filter: i16,
    ) -> Result<Option<(i16, crate::objects::masters::Task)>, GenieError> {
        let unit = &self.players.live(player, idx)?;
        let masters = self
            .masters
            .get(player)
            .ok_or(GenieError::unsupported("a work command: no master table"))?;
        let owner = &self.players.player(player)?;
        let actor = Actor {
            id: unit.id,
            master: unit.master(&self.masters, player)?,
            player,
            held: (unit.life.attribute_type, unit.life.amount),
            attribute_95: owner.attributes.get(95).copied().unwrap_or(0.0),
            civ: owner.civ,
        };
        let world = TaskWorld {
            players: &self.players,
            drop_sites: &self.drop_sites,
        };
        let (t, general) = if target == -1 {
            (None, None)
        } else {
            (
                Some(self.task_target(target, 1)?),
                Some(self.task_target(target, 2)?),
            )
        };
        let on = crate::unit_ai::task_finder::TaskOn {
            target: t.as_ref(),
            general: general.as_ref(),
            terrain: self.task_terrain(point),
        };
        Ok(masters
            .work_master(&actor, &on, filter, &world)?
            .map(|(with, task)| (with, *task)))
    }

    /// The terrain the task finder's general search reads at `point` (`FUN_1411a8ba0`,
    /// [`crate::unit_ai::task_finder::world::point_terrain`] on the map's terrain).
    pub(crate) fn task_terrain(&self, point: [f32; 3]) -> Option<i16> {
        crate::unit_ai::task_finder::world::point_terrain(self.spatial.terrain.as_ref(), point)
    }

    /// `LAB_1410706a8`: an action of the unit already on the `task` found takes the new target
    /// itself (its slot 9): a Hunt or Gather on the same task retargets, a Build or Repair on
    /// top takes the target again, a Transport on the same task works at the point
    /// ([`World::retake_transport`]); the base action types on top answer 0 (the factory
    /// follows).
    /// Answers whether an action kept the work.
    pub(super) fn retake_task(
        &mut self,
        player: usize,
        idx: usize,
        (target, point, clears): (i32, [f32; 3], bool),
        task: &crate::objects::masters::Task,
        same_master: bool,
    ) -> Result<bool, GenieError> {
        let unit = &self.players.live(player, idx)?;
        let top = unit.top_action_kind();
        Ok(match unit.actions.hunt().copied() {
            Some(h) if task.action_type == HUNT && h.task.id == task.id => {
                self.retarget_hunt_work(player, idx, (target, point, clears))?;
                true
            }
            _ if top == Some(task.action_type) && task.action_type == action_type::BUILD => {
                if !same_master {
                    return Err(GenieError::unsupported(
                        "a work command on the Build task of the unit's former master",
                    ));
                }
                self.retake_build(player, idx, target, point, clears)?;
                true
            }
            _ if top == Some(task.action_type)
                && task.action_type == crate::construction::repair::REPAIR =>
            {
                self.retake_repair(player, idx, target, point, clears)?;
                true
            }
            _ if top == Some(GATHER)
                && task.action_type == GATHER
                && unit.actions.gather().is_some_and(|g| g.task.id == task.id) =>
            {
                self.retarget_gather_work(player, idx, (target, point, clears))?
            }
            _ if top == Some(crate::garrison::transport::TRANSPORT)
                && task.action_type == crate::garrison::transport::TRANSPORT =>
            {
                self.retake_transport((player, idx), (target, point), task, clears)?
            }
            // the top action's slot 0x48 (FUN_140a977a0) on the same task: the base's
            // (FUN_140a7bf40: the MoveTo's, the Enter's, the missile's, the Fly's, the Make's, the
            // wait for a foundation's) answers 0, and the work goes on to the factory
            _ if top == Some(task.action_type) && top != Some(HUNT) => match top {
                Some(
                    crate::movement::move_to::MOVE_TO
                    | crate::garrison::ENTER
                    | crate::combat::missile::MISSILE
                    | crate::movement::fly::FLY
                    | crate::gathering::make::MAKE
                    | crate::construction::wait_solidify::WAIT_SOLIDIFY,
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
        })
    }

    /// A Hunt on the work's task retargets (slot 9, [`World::retarget_hunt`]) and its unit AI
    /// hears the hunt order and action on the target at the work's point; a hunt sent to its
    /// player's master 84 is not modelled.
    pub(super) fn retarget_hunt_work(
        &mut self,
        player: usize,
        idx: usize,
        (target, point, clears): (i32, [f32; 3], bool),
    ) -> Result<(), GenieError> {
        let t = self.task_target(target, 0)?;
        if t.master.id == MARKET && t.player == player {
            return Err(GenieError::unsupported(
                "a hunt sent to its player's master 84",
            ));
        }
        let class = i32::from(t.master.class);
        self.retarget_hunt(player, idx, target)?;
        let unit = self.players.live_mut(player, idx)?;
        if let Some(ai) = unit.ai.as_mut() {
            ai.notify_work(
                Some(OrderKind::Hunt),
                Some(ActionKind::Hunt),
                Some((target, class)),
                point,
                clears,
                self.streams.get(Stream::Ai),
            )?;
            self.remember_order(player, idx, (Some(OrderKind::Hunt), Some(ActionKind::Hunt)))?;
        }
        Ok(())
    }

    /// A Gather on the work's task retargets (slot 9, [`World::retarget_gather`]); when it took
    /// the target its unit AI hears the gather order and action on it at the work's point.
    /// Answers whether it took it; a gather sent to its player's master 84 is not modelled.
    pub(super) fn retarget_gather_work(
        &mut self,
        player: usize,
        idx: usize,
        (target, point, clears): (i32, [f32; 3], bool),
    ) -> Result<bool, GenieError> {
        let t = self.task_target(target, 0)?;
        if t.master.id == MARKET && t.player == player {
            return Err(GenieError::unsupported(
                "a gather sent to its player's master 84",
            ));
        }
        let class = i32::from(t.master.class);
        let took = self.retarget_gather(player, idx, target)?;
        if took {
            let unit = self.players.live_mut(player, idx)?;
            if let Some(ai) = unit.ai.as_mut() {
                ai.notify_work(
                    Some(OrderKind::Gather),
                    Some(ActionKind::Gather),
                    Some((target, class)),
                    point,
                    clears,
                    self.streams.get(Stream::Ai),
                )?;
                self.remember_order(
                    player,
                    idx,
                    (Some(OrderKind::Gather), Some(ActionKind::Gather)),
                )?;
            }
        }
        Ok(took)
    }

    /// The action list's factory (slot 0x10) for the work's `task` on `target`: the Enter,
    /// Attack, Hunt, Build, Repair, Gather or Transport starts; a mule cart's task makes no
    /// action (the move without one, [`World::work_move`]).
    pub(super) fn start_task(
        &mut self,
        player: usize,
        idx: usize,
        (target, point): (i32, [f32; 3]),
        task: &crate::objects::masters::Task,
        (clears, notified): (bool, bool),
    ) -> Result<(), GenieError> {
        match task.action_type {
            crate::garrison::ENTER => self.start_enter(player, idx, target, task, point, clears),
            ATTACK_TASK => self.start_attack(player, idx, target, task, point, clears),
            HUNT => self.start_hunt(HuntOrder {
                player,
                index: idx,
                target,
                task: HuntTask {
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
                notify: true,
            }),
            action_type::BUILD => self.start_build(player, idx, target, task, point, clears),
            crate::construction::repair::REPAIR => {
                self.start_repair(player, idx, target, task, point, clears)
            }
            // the factory makes no action of a mule cart's task: the move without one
            crate::construction::build::CART_TASK => {
                self.work_move(player, idx, target, point, (clears, notified))
            }
            GATHER => self.start_gather(player, idx, target, task, Some((point, clears))),
            crate::garrison::transport::TRANSPORT => {
                self.start_transport((player, idx), (target, point), task, clears)
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
        let unit = &self.players.live(player, idx)?;
        let master = unit.master(&self.masters, player)?;
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
            Some((target, i32::from(t.master(&self.masters, tp)?.class)))
        };
        let (order, action) = if alive {
            (Some(OrderKind::Move), Some(ActionKind::Move))
        } else {
            (None, None)
        };
        let unit = self.players.live_mut(player, idx)?;
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
        if alive || class == class::CLASS_51 {
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
    /// [`World::change_master`], or [`World::transform_building`] for a building) though it be its
    /// own. None: no task. The terrain under the point, which only a task for a terrain reads, is
    /// not passed.
    ///
    /// [`find_task`]: crate::unit_ai::task_finder::find_task
    pub(crate) fn find_work_task(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
        filter: i16,
        change: bool,
    ) -> Result<Option<crate::objects::masters::Task>, GenieError> {
        let unit = &self.players.live(player, idx)?;
        let own = unit.master(&self.masters, player)?;
        let owner = &self.players.player(player)?;
        let actor = Actor {
            id: unit.id,
            master: own,
            player,
            held: (unit.life.attribute_type, unit.life.amount),
            attribute_95: owner.attributes.get(95).copied().unwrap_or(0.0),
            civ: owner.civ,
        };
        let world = TaskWorld {
            players: &self.players,
            drop_sites: &self.drop_sites,
        };
        let t = self.task_target(target, 1)?;
        let general = self.task_target(target, 2)?;
        // slot 0x820's point: the target's place
        let at = self.object(target).map_or([0.0; 3], |(_, u)| u.position());
        let terrain = self.task_terrain(at);
        let find = |list: &Master, filter: i16| {
            crate::unit_ai::task_finder::find_task(
                list,
                &actor,
                Some(&t),
                Some(&general),
                terrain,
                filter,
                &world,
            )
            .map(Option::<&crate::objects::masters::Task>::copied)
        };
        if let Some(task) = find(own, filter)? {
            return Ok(Some(task));
        }
        if own.task_by_group == 0 {
            return Ok(None);
        }
        let mut found = None;
        // the master table exists: master_of found the unit's own in it
        for candidate in self.masters.at(player)?.iter() {
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
            if self.master(player, with)?.kind == BUILDING {
                self.transform_building(player, idx, with)?;
            } else {
                self.change_master(player, idx, with)?;
            }
        }
        Ok(Some(task))
    }
}
