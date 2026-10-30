//! A group's command (FD-036): `BUnitGroup::command` (`FUN_14100efd0`), which chooses by the
//! task a member has on the target how the group's order runs.

use crate::GenieError;
use crate::groups::GroupCommand;
use crate::groups::attack::AttackOrder;
use crate::groups::command::{ATTACK_COMMANDS, MOVE_COMMAND, MoveTo};
use crate::groups::task::TargetTask;
use crate::objects::ObjectId;
use crate::world::World;

/// The tasks `BUnitGroup::command` runs through the general command (`FUN_14100fc50`): the cases
/// of its switch besides the attack path's.
const GENERAL_COMMANDS: [i32; 11] = [3, 5, 0xc, 0x65, 0x69, 0x6a, 0x6e, 0x6f, 0x84, 0x88, 0x99];
/// What `BUnitGroup::command` is told besides the group, the target and the point: the order's
/// queued (`param_5`) and direct (`param_6`) bytes, its seventh byte (`param_7`), which the
/// group keeps as its target's `mDisableRetargeting`, and its task filter (`param_8`, `-1` for
/// any).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommandArgs {
    /// `param_5`: the command is queued.
    pub queued: bool,
    /// `param_6`: a unit with a unit AI takes it at once.
    pub direct: bool,
    /// `param_7`.
    pub disable_retargeting: bool,
    /// `param_8`.
    pub filter: i16,
}

/// The group's command on a task whose command is not modelled: the game's general command
/// (`FUN_14100fc50`) or a member's attack at once (`FUN_14102bbd0`).
pub(super) const TARGET_COMMAND_GAP: GenieError =
    GenieError::unsupported("a group's command on a target (its task, slot 0x130)");

impl World {
    /// `BUnitGroup::command` (`FUN_14100efd0`) of group `group` on `target` (-1 for none, or an
    /// object the world does not hold: the game passes a null object) at `point`, with the
    /// order's bytes ([`CommandArgs`]). Its rams are not re-added (`readdRams`, `FUN_14102b730`:
    /// `DAT_143f01837`, which nothing writes, is 0), flags bit 5 clears, and the first member the
    /// world holds with at least a hit point names the task on the target (slot 0x130,
    /// [`World::member_task`]: none without a target). A group of horde units moves as a horde
    /// (`commandHorde`, `FUN_141010180`, not modelled); any other notes whether the command was
    /// queued (`+0x13d`, [`crate::groups::UnitGroup::replaces_queue`]) and runs the command by the
    /// task's action type: without a task, or for 1, a move ([`World::command_move_to`] with its
    /// target forgotten); for 1, 7, 9, `0x68` and `0x7d` the attack path
    /// ([`World::command_attack`], or a queue entry); the other tasks' general command is not
    /// modelled. A move begun stops the members (`stopAllUnits` with 0, [`World::stop_all_units`]),
    /// and flags bit 17 clears. True when the move began.
    ///
    /// # Errors
    ///
    /// A horde, the general command, and what the paths do not model.
    pub fn group_command(
        &mut self,
        group: i32,
        target: i32,
        point: [f32; 2],
        order: CommandArgs,
    ) -> Result<bool, GenieError> {
        let g = self.groups.held_mut(group)?;
        g.flags &= !0x20;
        let held = ObjectId::of_field(target).filter(|t| self.world_object(t.get()).is_some());
        let target = ObjectId::field(held);
        let task = match held {
            Some(t) => self.group_task(group, t, point, order)?,
            None => None,
        };
        if self.all_horde_units(group)? {
            return Err(GenieError::unsupported(
                "a horde's command (commandHorde, FUN_141010180)",
            ));
        }
        let g = self.groups.held_mut(group)?;
        g.replaces_queue = u8::from(!order.queued);
        let command = task.map(|t| t.command);
        let mut moved = false;
        if command.is_none_or(|c| i32::from(c) == MOVE_COMMAND) {
            moved = self.command_move_to(
                group,
                target,
                point,
                MoveTo {
                    forget_target: true,
                    queued: order.queued,
                    direct: order.direct,
                    ..MoveTo::default()
                },
            )?;
        }
        if let (Some(task), Some(held)) = (task, held) {
            if ATTACK_COMMANDS.contains(&i32::from(task.command)) {
                moved = self.attack_command(group, held, point, (task, order), moved)?;
            } else if GENERAL_COMMANDS.contains(&i32::from(task.command)) {
                return Err(TARGET_COMMAND_GAP);
            }
        }
        if moved {
            self.stop_all_units(group, false)?;
        }
        let g = self.groups.held_mut(group)?;
        g.flags &= !0x2_0000;
        Ok(moved)
    }

    /// `BUnitGroup::command`'s attack tasks (1, 7, 9, `0x68`, `0x7d`) on `target`, `moved` what
    /// task 1's move before it returned: a command not queued either joins the command queue
    /// (not direct, with commands queued: the queue is approved and run, true) or runs
    /// [`World::command_attack`] after the queue empties when the last command was not queued; a
    /// queued one joins the queue (approved when direct), a group without a command that is told
    /// directly hibernating, and answers what the move before it did.
    fn attack_command(
        &mut self,
        group: i32,
        target: ObjectId,
        point: [f32; 2],
        (task, order): (TargetTask, CommandArgs),
        moved: bool,
    ) -> Result<bool, GenieError> {
        let g = self.groups.held_mut(group)?;
        let entry = |approved: u8| GroupCommand {
            command: i32::from(task.command),
            target_id: target.get(),
            range: task.range.unwrap_or(0.0),
            point: [point[0], 0.0, point[1]],
            approved,
            disable_retargeting: u8::from(order.disable_retargeting),
            approved_2: order.filter as u16,
        };
        if order.queued {
            g.queue_command(entry(u8::from(order.direct)));
            if g.current_command == -1 && order.direct && g.hibernating != 1 {
                g.hibernating = 1;
            }
            return Ok(moved);
        }
        if !order.direct && !g.command_queue.is_empty() {
            g.queue_command(entry(0));
            g.approve_queued();
            self.check_queued_orders(group)?;
            return Ok(true);
        }
        if g.replaces_queue != 0 {
            g.clear_command_queue();
        }
        self.command_attack(
            group,
            target,
            AttackOrder {
                disable_retargeting: order.disable_retargeting,
                filter: order.filter,
            },
        )
    }
}
