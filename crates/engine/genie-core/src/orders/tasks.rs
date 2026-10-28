//! The hunt and gather commands, a build's and a repair's start, retake and target.

use crate::GenieError;
use crate::gathering::hunt::HuntTask;
use crate::math::rng::Stream;
use crate::orders::{BUILDING, HUNT_TARGET_GONE, NO_HUNT_TASK, Place};
use crate::unit_ai::ActionKind;
use crate::unit_ai::OrderKind;
use crate::world::World;

impl World {
    /// `HuntObject`'s unit part (`FUN_140c4e220`, [`crate::unit_ai::Effect::HuntObject`], after the
    /// unit's update): its stopAction (slot 0x3c0: the action list cleared,
    /// [`World::clear_actions`], and in state 2 the standing sprite), then its hunt command (object
    /// slot 0x360, `FUN_1412151f0`, forced): with no action left slot 0x820 finds the task
    /// ([`World::find_work_task`] with any filter and the master switch) and a new Hunt on it,
    /// whatever its action type or requirement (`FUN_140ac2dd0`), replaces the unit's actions
    /// ([`World::start_hunt`] without the unit AI's notify: `HuntObject` set the AI itself). A
    /// target the world does not hold or no task would fail the command, which the unit AI took
    /// as done: not modelled.
    pub(crate) fn hunt_command(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
    ) -> Result<(), GenieError> {
        self.clear_actions(player, idx)?;
        if self.players[player].objects[idx].life.state == 2 {
            self.standing_sprite(player, idx)?;
        }
        let (_, t) = self.object(target).ok_or(HUNT_TARGET_GONE)?;
        let point = t.position;
        let task = self
            .find_work_task(player, idx, target, -1, true)?
            .ok_or(NO_HUNT_TASK)?;
        self.start_hunt(
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
            false,
            false,
        )
    }

    /// The unit's gather command (object slot 0x368, `FUN_141085030`) forced on `target`, as the
    /// unit AI's `GatherObject` runs it ([`crate::unit_ai::Effect::GatherObject`], after the
    /// unit's update): slot 0x820 finds the task ([`World::find_work_task`] with any filter and
    /// the master switch);
    /// the unit's top Gather on the task found takes the target (its slot 0x48 with the target's
    /// place, [`World::retarget_gather`]; what it answers the command passes over); else a new
    /// Gather on the task replaces the unit's actions (`FUN_140a88310`, `FUN_140a98620`, slot
    /// 0x808: [`World::start_gather`] without the unit AI's notify), whatever the task's action
    /// type (a farm's foundation: the builder's Build task). A target the world does not hold or
    /// no task would fail the command, which the unit AI took as done: not modelled.
    pub(crate) fn gather_command(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
    ) -> Result<(), GenieError> {
        if self.object(target).is_none() {
            return Err(GenieError::unsupported(
                "a gather command on an object the world does not hold",
            ));
        }
        let task = self
            .find_work_task(player, idx, target, -1, true)?
            .ok_or(GenieError::unsupported("a gather command without a task"))?;
        let unit = &self.players[player].objects[idx];
        if unit.top_action_kind() == Some(crate::gathering::gather::GATHER)
            && unit.actions.gather().is_some_and(|g| g.task.id == task.id)
        {
            let _ = self.retarget_gather(player, idx, target)?;
            return Ok(());
        }
        // FUN_140a88310 makes a Gather on whatever task it found (a farm's foundation gives the
        // builder's Build task)
        self.start_gather(player, idx, target, &task, None)
    }

    /// The work command's Build (its task of action type 101, the factory `FUN_140aca4b0` case
    /// 0x65): the action takes its foundation (slot 0x178); the unit AI's notify
    /// (`FUN_14106e4e0` kind 0x65: the build order and action, the foundation with its class, the
    /// work's point); the install (slot 0x808) clears the action list and holds the Build
    /// ([`crate::construction::build::BuildAction::start`] with the footprints' gap, the task's range and work
    /// value, the master's work rate with the unit's powerup), as BUILD's builders take it.
    pub(super) fn start_build(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
        task: &crate::objects::masters::Task,
        point: [f32; 3],
        clears: bool,
    ) -> Result<(), GenieError> {
        let (tp, t) = self.object(target).ok_or(GenieError::unsupported(
            "a Build's foundation the slice does not hold",
        ))?;
        let tm = self.master_of(tp, t.life.master_id)?;
        let (t_at, t_radius, class) = (t.position, tm.radius, i32::from(tm.class));
        let unit = &self.players[player].objects[idx];
        let builder = self.master_of(player, unit.life.master_id)?;
        let gap = crate::movement::footprint_gap(unit.position, builder.radius, t_at, t_radius);
        let build_task = crate::construction::build::BuildTask {
            range: task.work_range,
            work_value: task.work_val1,
            work_rate: builder.work_rate + unit.work_rate_powerup,
            resource: task.attribute_type,
            id: task.id,
        };
        self.take(target, self.releaser(player, idx));
        let unit = &mut self.players[player].objects[idx];
        if let Some(ai) = unit.ai.as_mut() {
            ai.notify_work(
                Some(OrderKind::Build),
                Some(ActionKind::Build),
                Some((target, class)),
                point,
                clears,
                self.streams.get(Stream::Ai),
            )?;
            self.remember_order(
                player,
                idx,
                (Some(OrderKind::Build), Some(ActionKind::Build)),
            )?;
        }
        self.clear_actions(player, idx)?;
        self.players[player].objects[idx].actions.set_build(Some(
            crate::construction::build::BuildAction::start(target, t_at, gap, build_task),
        ));
        Ok(())
    }

    /// The Repair's constructor (`FUN_140ad7fa0`) on `target`, which the task finder found the
    /// task `task` on: the action takes its target (slot 0x178) at its place (a ship's meeting
    /// point, `FUN_140ad9120`, is not modelled: [`crate::construction::repair::SHIP_REPAIR`]); its start
    /// (slot 5, `FUN_140ad9030` with its force flag) walks to the target (state 4) within the
    /// unit's master's weapon range ([`crate::construction::repair::RepairAction::start`]). With the target's
    /// class.
    pub(super) fn new_repair(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
        task: &crate::objects::masters::Task,
    ) -> Result<(crate::construction::repair::RepairAction, i32), GenieError> {
        let (tp, t) = self.object(target).ok_or(GenieError::unreachable(
            "a Repair's target the world does not hold",
        ))?;
        let tm = self.master_of(tp, t.life.master_id)?;
        if crate::construction::repair::SHIPS.contains(&tm.class) {
            return Err(crate::construction::repair::SHIP_REPAIR);
        }
        let (t_at, class) = (t.position, i32::from(tm.class));
        let unit = &self.players[player].objects[idx];
        let range = self.master_of(player, unit.life.master_id)?.weapon_range;
        self.take(target, self.releaser(player, idx));
        let repair_task = crate::construction::repair::RepairTask {
            id: task.id,
            work_value: task.work_val1,
        };
        Ok((
            crate::construction::repair::RepairAction::start(target, t_at, repair_task, range),
            class,
        ))
    }

    /// The work command's Repair (its task of action type 106, the factory `FUN_140aca4b0` case
    /// 0x6a, [`World::new_repair`]); the unit AI's notify (`FUN_14106e4e0` kind 0x6a: the repair
    /// order 0x2ce and action 0x26a, the target with its class, the work's point); the install
    /// (slot 0x808) clears the action list and holds the Repair.
    pub(super) fn start_repair(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
        task: &crate::objects::masters::Task,
        point: [f32; 3],
        clears: bool,
    ) -> Result<(), GenieError> {
        // the task finder found the task on the target, which the world holds
        let (repair, class) = self.new_repair(player, idx, target, task)?;
        let unit = &mut self.players[player].objects[idx];
        if let Some(ai) = unit.ai.as_mut() {
            ai.notify_work(
                Some(OrderKind::Repair),
                Some(ActionKind::Repair),
                Some((target, class)),
                point,
                clears,
                self.streams.get(Stream::Ai),
            )?;
            self.remember_order(
                player,
                idx,
                (Some(OrderKind::Repair), Some(ActionKind::Repair)),
            )?;
        }
        self.clear_actions(player, idx)?;
        self.players[player].objects[idx]
            .actions
            .set_repair(Some(repair));
        Ok(())
    }

    /// The unit's repair command its unit AI gave ([`crate::unit_ai::world_view::RepairOrder`]), after the
    /// update: the Repair on top takes the target ([`World::repair_retarget`]); else the master
    /// switches (slot 0x90 with 0, [`World::change_master`]), then a Repair on the task found
    /// ([`World::new_repair`]) replaces the unit's actions (`FUN_140a98620`, slot 0x808).
    ///
    /// # Errors
    ///
    /// What the master change, the retarget and the Repair do not model.
    pub(crate) fn repair_order(
        &mut self,
        player: usize,
        idx: usize,
        order: crate::unit_ai::world_view::RepairOrder,
    ) -> Result<(), GenieError> {
        match order {
            crate::unit_ai::world_view::RepairOrder::Retake(target) => {
                self.repair_retarget(player, idx, target).map(|_class| ())?;
            }
            crate::unit_ai::world_view::RepairOrder::Start {
                target,
                switches,
                task,
            } => {
                for master in switches.into_iter().flatten() {
                    self.change_master(player, idx, master)?;
                }
                if let Some(task) = task {
                    let (repair, _) = self.new_repair(player, idx, target, &task)?;
                    self.clear_actions(player, idx)?;
                    self.players[player].objects[idx]
                        .actions
                        .set_repair(Some(repair));
                }
            }
        }
        Ok(())
    }

    /// The work command's target for the Repair the unit runs ([`World::repair_retarget`]); the
    /// work command then tells the unit AI (slot 0x828: order 0x2ce, action 0x26a, the target
    /// with its class, the work's point).
    pub(super) fn retake_repair(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
        point: [f32; 3],
        clears: bool,
    ) -> Result<(), GenieError> {
        let class = self.repair_retarget(player, idx, target)?;
        let unit = &mut self.players[player].objects[idx];
        if let Some(ai) = unit.ai.as_mut() {
            ai.notify_work(
                Some(OrderKind::Repair),
                Some(ActionKind::Repair),
                Some((target, class)),
                point,
                clears,
                self.streams.get(Stream::Ai),
            )?;
            self.remember_order(
                player,
                idx,
                (Some(OrderKind::Repair), Some(ActionKind::Repair)),
            )?;
        }
        Ok(())
    }

    /// The Repair on top takes `target` (its slot 0x48, `FUN_140adb480`): its own target again
    /// changes nothing while it faces it (state 6), nor for a computer player's repairer walking
    /// to it (4); else slot 0x68 takes the target (`FUN_140ada470`: a repairer counted among
    /// another target's repairers leaves them; the base lets the old one go and takes the new,
    /// [`World::retarget_action`]) at its place (a ship's meeting point is not modelled,
    /// [`crate::construction::repair::SHIP_REPAIR`]) and `set_state(4)` walks to it again (a still counted
    /// repairer leaves the repairers, the `MoveTo` goes: the unit stops). The slot answers 1.
    /// With the target's class.
    pub(super) fn repair_retarget(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
    ) -> Result<i32, GenieError> {
        let (tp, t) = self.object(target).ok_or(GenieError::unreachable(
            "a Repair's target the world does not hold",
        ))?;
        let tm = self.master_of(tp, t.life.master_id)?;
        let (t_at, class, ship) = (
            t.position,
            i32::from(tm.class),
            crate::construction::repair::SHIPS.contains(&tm.class),
        );
        let computer = self.players[player].computer;
        let repair = self.players[player].objects[idx]
            .actions
            .repair()
            .copied()
            .expect("the unit's top action");
        let same = repair.target == target;
        let stays = same && (repair.state == 6 || (repair.state == 4 && computer));
        if !stays {
            if !same && repair.counted {
                self.leave_builders(repair.target)?;
            }
            let taken = self.retarget_action(player, idx, repair.target, target, false)?;
            if ship {
                return Err(crate::construction::repair::SHIP_REPAIR);
            }
            let unit = &mut self.players[player].objects[idx];
            let r = unit.actions.repair_mut().expect("the unit's top action");
            // slot 0x68 dropped the count on another target; set_state(4)'s prelude makes a
            // repairer still counted leave its repairers
            let counted = same && r.counted;
            r.counted = false;
            r.target = taken;
            r.point = t_at;
            r.walk_again(unit.body.mover_mut())?;
            if counted {
                self.leave_builders(target)?;
            }
        }
        Ok(class)
    }

    /// The work command's target for the Build the unit runs (`LAB_1410706a8`: its task is the
    /// one found, slot 0x48 takes the target, `FUN_140ab1b30`), then the notify of the Build
    /// (slot 0x828: order 702, action 602, the target with its class). Its own foundation again
    /// is [`crate::construction::build::BuildAction::retake`]; another foundation of the player's (slot 0x68,
    /// `FUN_140ab0300`: a builder counted among the old one's builders leaves them, through
    /// `FUN_140aaddb0`, an annex's main building's; `FUN_140a7a230` lets the old one go, slot
    /// 0x180, and takes the new, slot 0x178) starts over in `set_state(4)`
    /// ([`crate::construction::build::BuildAction::retarget`]). Another player's object (`set_state(11)`) is
    /// not modelled.
    pub(super) fn retake_build(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
        point: [f32; 3],
        clears: bool,
    ) -> Result<(), GenieError> {
        let class = self.build_take(player, idx, target)?;
        let unit = &mut self.players[player].objects[idx];
        if let Some(ai) = unit.ai.as_mut() {
            ai.notify_work(
                Some(OrderKind::Build),
                Some(ActionKind::Build),
                Some((target, class)),
                point,
                clears,
                self.streams.get(Stream::Ai),
            )?;
            self.remember_order(
                player,
                idx,
                (Some(OrderKind::Build), Some(ActionKind::Build)),
            )?;
        }
        Ok(())
    }

    /// The Build's slot 0x48 (`FUN_140ab1b30`) on `target` ([`World::retake_build`]'s part
    /// before the notify): returns the target's master class.
    pub(crate) fn build_take(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
    ) -> Result<i32, GenieError> {
        let (tp, t) = self.object(target).ok_or(GenieError::unsupported(
            "a Build's foundation the slice does not hold",
        ))?;
        if tp != player {
            return Err(GenieError::unsupported(
                "a Build retargeted to another player's object (set_state 11)",
            ));
        }
        let tm = self.master_of(tp, t.life.master_id)?;
        let (t_at, t_radius, class) = (t.position, tm.radius, i32::from(tm.class));
        let unit = &self.players[player].objects[idx];
        let old = unit
            .actions
            .build()
            .copied()
            .ok_or(GenieError::unsupported(
                "a Build's retarget without its Build",
            ))?;
        let builder = self.master_of(player, unit.life.master_id)?;
        let gap = crate::movement::footprint_gap(unit.position, builder.radius, t_at, t_radius);
        if old.target != target {
            // the task finder took the new target below state 3, which slot 0x68 keeps
            let _ = self.build_set_target(player, idx, (old.target, old.added), target)?;
            let unit = &mut self.players[player].objects[idx];
            let (Some(build), mover) = (unit.actions.build_mut(), unit.body.mover_mut()) else {
                return Err(GenieError::unsupported(
                    "a Build's retarget without its Build",
                ));
            };
            build.retarget((target, t_at), gap, mover)?;
            return Ok(class);
        }
        let computer = self.players[player].computer;
        let Some(Place::Live(fp, fi)) = self.place(target) else {
            return Err(GenieError::unsupported("a Build's foundation that sleeps"));
        };
        let (mut build, mut mover) = {
            let unit = &mut self.players[player].objects[idx];
            (unit.actions.take_build(), unit.body.take_mover())
        };
        // FUN_140aaddb0's building: an annex's main building
        let counted = match self.counted_building(target)? {
            Some(id) if id != target => self.object_mut(id).map(|(_, b)| b),
            _ => Some(&mut self.players[fp].objects[fi]),
        };
        let builders = counted
            .and_then(|b| b.construction.as_mut())
            .map(|k| &mut k.builders);
        let r = build.as_mut().expect("found above").retake(
            (t_at, gap),
            mover.as_mut(),
            builders,
            computer,
        );
        let unit = &mut self.players[player].objects[idx];
        unit.actions.set_build(build);
        unit.body.put_mover(mover);
        r?;
        Ok(class)
    }

    /// The Build's slot 0x68 (`FUN_140ab0300`) of the builder at `idx`, its target `old` and
    /// whether it is counted among the builders (`added`, `+0x68`), on `new` (−1 for none): a
    /// counted Build of another target leaves the builders ([`World::leave_builders`]) and is no
    /// longer counted; then the base ([`World::retarget_action`]) takes the target. Returns the
    /// Build's target after it.
    pub(crate) fn build_set_target(
        &mut self,
        player: usize,
        idx: usize,
        (old, added): (i32, bool),
        new: i32,
    ) -> Result<i32, GenieError> {
        let leaves = old != new && added;
        if leaves {
            self.leave_builders(old)?;
        }
        let target = self.retarget_action(player, idx, old, new, false)?;
        if let Some(build) = self.players[player].objects[idx].actions.build_mut() {
            build.target = target;
            build.added &= !leaves;
        }
        Ok(target)
    }

    /// `FUN_140aaddb0`: the building a Build on `id` counts its builder in, when the world holds
    /// `id` (live or sleeping, `FUN_1415417a0`): the object itself, or for a building (type 80)
    /// that is an annex (`+0xbb8`) its main building.
    pub(crate) fn counted_building(&self, id: i32) -> Result<Option<i32>, GenieError> {
        let Some((p, f)) = self.object(id) else {
            return Ok(None);
        };
        let kind = self.master_of(p, f.life.master_id)?.kind;
        Ok(Some(if kind == BUILDING {
            f.annex_of.unwrap_or(id)
        } else {
            id
        }))
    }

    /// The builder count: the building `FUN_140aaddb0` gives for the foundation `id` a Build
    /// counted its builder among ([`World::counted_building`]; a Repair's target the same,
    /// `FUN_140ad9050`) counts one less (`+0x9e8`: a building's in its construction record, a
    /// unit's its own); none once the world no longer holds `id`. Outside an update
    /// ([`crate::world::lets::Lets::leave_builders`] on the whole world).
    pub(crate) fn leave_builders(&mut self, id: i32) -> Result<(), GenieError> {
        self.with_lets(0, |lets, others| lets.leave_builders(others, id))
    }
}
