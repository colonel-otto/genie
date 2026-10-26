//! A group's orders (FD-036): `BUnitGroup::command` (`FUN_14100efd0`), `commandMoveTo`
//! (`FUN_141010a80`) and `stopAllUnits` (`FUN_14103a0d0`), the HD PDB's names.

use crate::GenieError;
use crate::group::{GroupCommand, SpecificMove, UnitGroup};
use crate::movement::PathData;
use crate::order::Place;
use crate::tile_path::Path;
use crate::turn::Turns;

/// A group whose top speed is below this does not move (`commandMoveTo`, `DAT_142eb0c60`); nor is
/// a path shorter than it one.
const EPSILON: f32 = crate::math::image::group::EPSILON;
/// The group's command while it moves (`mCurrentCommand` 1).
pub const MOVE_COMMAND: i32 = 1;
/// A new path's flags (`FUN_14152eea0`'s construction).
const NEW_PATH_FLAGS: u32 = 6;
/// The plans a path level may still try (`FUN_140f9b800`, `FUN_140f98e80`).
const CONTINUE: i32 = 10;
/// A patrol's leg goes back and forth when its point is at least this far from the group on
/// the ground (`commandMoveTo` at 0x141010ddb, `DAT_14330ebb8`).
const PATROL_LEG_REACH: f32 = crate::math::image::group::PATROL_LEG_REACH;

/// The queued commands on a target `checkQueuedOrders` hands to `FUN_14100fc50` when the world
/// holds the target (its switch's cases 3 and `0x2713`).
const TARGET_COMMANDS: [i32; 11] = [3, 5, 0xc, 0x65, 0x69, 0x6a, 0x6e, 0x6f, 0x84, 0x88, 0x2713];
/// The queued attacks `checkQueuedOrders` hands to `FUN_14100f7f0` when the world holds the
/// target.
const ATTACK_COMMANDS: [i32; 4] = [7, 9, 0x68, 0x7d];

/// What `commandMoveTo` is told besides the target and the point: the range (`param_5`), and its
/// flags `param_6` (a patrol's leg), `param_7` (the group's target goes), `param_8`, `param_9`
/// (queued), `param_10` (at once, though orders are queued) and `param_11`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MoveTo {
    /// `param_5`: the path level's range.
    pub range: f32,
    /// `param_6`: a patrol's leg, whose path loops.
    pub leg: bool,
    /// `param_7`: the group forgets its target (`mCurrentTarget`).
    pub forget_target: bool,
    /// `param_8`.
    pub hold: bool,
    /// `param_9`: the move goes into the order queue.
    pub queued: bool,
    /// `param_10`: the move runs though orders are queued.
    pub direct: bool,
    /// `param_11`.
    pub keep_flag: bool,
}

impl UnitGroup {
    /// `queueOrder` (`FUN_14102b640`): a command at the queue's end (`+0x420`, its count
    /// `+0x450`) of `command`, `target`, `point`, `range` and its approved byte,
    /// retargeting byte and 16 bits after them.
    #[allow(clippy::too_many_arguments)]
    pub fn queue_command(
        &mut self,
        command: i32,
        target: i32,
        point: [f32; 3],
        range: f32,
        approved: u8,
        disable_retargeting: u8,
        approved_2: u16,
    ) {
        self.command_queue.push(GroupCommand {
            command,
            target_id: target,
            range,
            point,
            approved,
            disable_retargeting,
            approved_2,
        });
    }

    /// `FUN_14100dd10`: the queue's approved commands go and the others turn approved.
    pub fn approve_queued(&mut self) {
        self.command_queue.retain(|c| c.approved == 0);
        for c in &mut self.command_queue {
            c.approved = 1;
        }
    }

    /// `FUN_14100f6b0`: flags bit 5 clears; with `forget_target` the target goes (-1, type -1,
    /// its place, range and radius 0, its retargeting byte 0); the rams are not re-added
    /// (`FUN_14102b730`, gated by `DAT_143f01837`, which nothing writes); every member's seconds
    /// since attacking are `f32::MAX`; the waypoints (`+0x398`, count `+0x3a0`) take (`x`, the
    /// desired height, `z`). The array's growth (5, then twice) fails only without memory.
    pub fn add_waypoint(&mut self, x: f32, z: f32, forget_target: bool) {
        self.flags &= !0x20;
        if forget_target {
            let t = &mut self.current_target;
            t.target = -1;
            t.kind = -1;
            t.original_position = [0.0; 3];
            t.range = 0.0;
            t.target_radius = 0.0;
            t.disable_retargeting = 0;
        }
        for m in &mut self.members {
            m.seconds_since_attacking = f32::MAX;
        }
        self.waypoints.push([x, self.desired_height, z]);
    }
}

impl Turns {
    /// `allHordeUnits` (`FUN_141008e50`): no member the world holds is of a master above type 69
    /// with a formation category (`+0x8bc`).
    ///
    /// # Errors
    ///
    /// A member of a master the slice does not have.
    ///
    /// # Panics
    ///
    /// When the world does not hold `group`: the game's routine gets the group itself, from a
    /// caller that holds it.
    pub fn all_horde_units(&self, group: i32) -> Result<bool, GenieError> {
        let g = self.groups.get(group).expect("the caller holds the group");
        self.horde_units_only(g)
    }

    /// [`Turns::all_horde_units`] of group `g`.
    ///
    /// # Errors
    ///
    /// A member of a master the slice does not have.
    pub fn horde_units_only(&self, g: &UnitGroup) -> Result<bool, GenieError> {
        for m in &g.members {
            if let Some((p, u)) = self.object(m.id) {
                let master = self.master_of(p, u.life.master_id)?;
                if 69 < master.kind && master.formation_category != 0 {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

    /// `BUnitGroup::command` (`FUN_14100efd0`) of group `group` on `target` (-1 for none) at
    /// `point`, `queued` (`param_5`, MOVE's `+0x13`) and `direct` (`param_6`, `+0x14`): its rams
    /// are not re-added (`readdRams`, `FUN_14102b730`: `DAT_143f01837`, which nothing writes, is
    /// 0), flags bit 5 clears, and the first member the world holds with at least 1 hit point
    /// names the task on the target (slot 0x130, `FUN_14106b8b0`: none without one). A group of
    /// horde units moves as a horde (`commandHorde`, `FUN_141010180`, not modelled); any other
    /// notes whether the command was queued (`+0x13d`, [`crate::group::UnitGroup::replaces_queue`]) and,
    /// without a task, moves ([`Turns::command_move_to`] with its target forgotten); a move begun
    /// stops the members (`stopAllUnits` with 0, [`Turns::stop_all_units`]), and flags bit 17
    /// clears. True when the move began. A task on a target is not modelled.
    ///
    /// # Errors
    ///
    /// A target, a horde, or what the move does not model.
    ///
    /// # Panics
    ///
    /// When the world does not hold `group`: the game's routine gets the group itself, from a
    /// caller that holds it.
    pub fn group_command(
        &mut self,
        group: i32,
        target: i32,
        point: [f32; 2],
        (queued, direct): (bool, bool),
    ) -> Result<bool, GenieError> {
        let g = self
            .groups
            .get_mut(group)
            .expect("the caller holds the group");
        g.flags &= !0x20;
        if target != -1 {
            return Err(GenieError::unsupported(
                "a group's command on a target (its task, slot 0x130)",
            ));
        }
        if self.all_horde_units(group)? {
            return Err(GenieError::unsupported(
                "a horde's command (commandHorde, FUN_141010180)",
            ));
        }
        let g = self.groups.get_mut(group).expect("the group found above");
        g.replaces_queue = u8::from(!queued);
        let moved = self.command_move_to(
            group,
            target,
            point,
            MoveTo {
                forget_target: true,
                queued,
                direct,
                ..MoveTo::default()
            },
        )?;
        if moved {
            self.stop_all_units(group, false)?;
        }
        let g = self.groups.get_mut(group).expect("the group found above");
        g.flags &= !0x2_0000;
        Ok(moved)
    }

    /// [`Turns::command_move_to`]'s queue: a queued move joins the group's command queue (direct
    /// as its flag; a group without a command that is told directly hibernates) and is not taken
    /// (`Some(false)`); an indirect move while commands are queued joins the queue too, which is
    /// approved and run ([`Turns::check_queued_orders`], `Some(true)`). None: the move runs now.
    ///
    /// # Errors
    ///
    /// A queued command the queue cannot run.
    ///
    /// # Panics
    ///
    /// When the world does not hold `group`.
    fn queue_move(
        &mut self,
        group: i32,
        point: [f32; 2],
        how: &MoveTo,
    ) -> Result<Option<bool>, GenieError> {
        let g = self
            .groups
            .get_mut(group)
            .expect("the caller holds the group");
        let at = [point[0], 0.0, point[1]];
        if how.queued {
            g.queue_command(
                MOVE_COMMAND,
                -1,
                at,
                how.range,
                u8::from(how.direct),
                0,
                0xffff,
            );
            if g.current_command == -1 && how.direct && g.hibernating != 1 {
                g.hibernating = 1;
            }
            return Ok(Some(false));
        }
        if !how.direct && !g.command_queue.is_empty() {
            g.queue_command(MOVE_COMMAND, -1, at, how.range, 0, 0, 0xffff);
            g.approve_queued();
            self.check_queued_orders(group)?;
            return Ok(Some(true));
        }
        Ok(None)
    }

    /// `commandMoveTo` (`FUN_141010a80`) of group `group` to `point` (`target` -1 for none): flags
    /// bits 5 and 12 clear; a changed target resets the attack plan's entries (`FUN_140fd76a0`
    /// on `+0x3b0`, `mCurrentAttackPlan`, which no save holds, every load empties and only a
    /// group's attack fills, which the slice does not run: empty here). A queued move goes into
    /// the command queue ([`UnitGroup::queue_command`], approved with `direct`) and, for a group
    /// without a command, `direct` makes it hibernate (`+0x13c` 1); false. A move behind queued
    /// commands without `direct` joins them unapproved, the approved ones go
    /// ([`UnitGroup::approve_queued`]) and the queue runs ([`Turns::check_queued_orders`]); true.
    /// Else a group whose last command was not queued empties its command queue
    /// ([`UnitGroup::clear_command_queue`]); `forget_target` clears its target (`mCurrentTarget`:
    /// -1 and type -1, its place, range, radius and retargeting byte 0); its rams are not
    /// re-added (`readdRams`, `FUN_14102b730`, gated by `DAT_143f01837`, which nothing writes).
    /// Each member's seconds since attacking are `f32::MAX`. A horde moves as one
    /// (`commandHorde`, not modelled). A changed target, or a patrol's path (the top path's flags
    /// bit 0, [`crate::group::formation::top_path`], or flags bit 11) when neither `leg` nor
    /// `hold`, sets flags `0xc000`. Below [`EPSILON`] of top speed nothing moves (false). The
    /// path (a new one, flags [`NEW_PATH_FLAGS`], made now) runs from the group's place through
    /// its waypoints (which it empties) to the point at the group's desired height, its flags
    /// bit 0 off and bit 1 on; a patrol's leg (`leg`) loops instead (bit 0 on) and goes back and
    /// forth (bit 1) when the point is at least [`PATROL_LEG_REACH`] from the group on the ground
    /// (`COMISS`/`JBE` at 0x141010de3: a NaN distance too). Flags bit 11 ends set only with
    /// `hold`, which also clears the path's bit 0 (`keep_flag`'s bit 11 is cleared again after
    /// it, and a leg leaves it to `hold` alone). Measured, under two points
    /// or at most [`EPSILON`] of length (`COMISS`/`JC` at 0x141010e2a: a NaN length keeps it) it
    /// is none. The group's path data resets ([`PathData`]'s `FUN_140f98e80`) and takes it
    /// (`FUN_140f9b800`: range, target, waypoint 1, state 2) as waypoint level 2 (that level even
    /// without a path); the members stop ([`Turns::stop_all_units`]) and the unit AIs of those
    /// the world's table holds, sleeping ones too, drop their notifies at the world's time
    /// (`FUN_140c5dc10`); the target's place is the point, the command [`MOVE_COMMAND`]. With
    /// motion flags bit 1 clear and no patrol the formation follows (`UpdateDesiredPositions`: a
    /// column without a path or when the player's `mMinColumnDistance` is below its length,
    /// `COMISS`/`JNC` at 0x141010fc8, a NaN too; else a line). Flags bit 8 is set. True.
    ///
    /// # Errors
    ///
    /// A horde, a queued command the queue cannot run ([`Turns::check_queued_orders`]),
    /// or what the members' stop and the formation do not model.
    ///
    /// # Panics
    ///
    /// When the world does not hold `group`: the game's routine gets the group itself, from a
    /// caller that holds it.
    pub fn command_move_to(
        &mut self,
        group: i32,
        target: i32,
        point: [f32; 2],
        how: MoveTo,
    ) -> Result<bool, GenieError> {
        let world_time = self.world_time;
        let g = self
            .groups
            .get_mut(group)
            .expect("the caller holds the group");
        g.flags &= !0x1020;
        let changed = if target == -1 {
            g.current_target.target != -1
        } else {
            target != g.current_target.target
        };
        if let Some(done) = self.queue_move(group, point, &how)? {
            return Ok(done);
        }
        let g = self.groups.get_mut(group).expect("the group found above");
        if g.replaces_queue != 0 {
            g.clear_command_queue();
        }
        if how.forget_target {
            let t = &mut g.current_target;
            t.target = -1;
            t.kind = -1;
            t.original_position = [0.0; 3];
            t.range = 0.0;
            t.target_radius = 0.0;
            t.disable_retargeting = 0;
        }
        for m in &mut g.members {
            m.seconds_since_attacking = f32::MAX;
        }
        if self.all_horde_units(group)? {
            return Err(GenieError::unsupported(
                "a horde's move (commandHorde, FUN_141010180)",
            ));
        }
        let g = self.groups.get_mut(group).expect("the group found above");
        // FUN_141046500: the top ancestor's path, else the walked level's
        let patrol = crate::group::formation::top_path(&g.motion.paths)
            .is_some_and(|l| l.path_flags & 1 != 0)
            || g.flags & 0x800 != 0;
        if (!how.hold && !how.leg && patrol) || changed {
            g.flags |= 0xc000;
        }
        // the top speed below the tolerance (`COMISS`/`JA` at 0x141010cc3)
        if g.motion.maximum_velocity_forward < EPSILON {
            return Ok(false);
        }
        let mut path = Path::default();
        let _ = path.push(g.motion.position[0]);
        for &w in &g.waypoints {
            let _ = path.push(w);
        }
        g.waypoints.clear();
        let at = [point[0], g.desired_height, point[1]];
        let _ = path.push(at);
        let mut path_flags = move_path_flags(g.motion.position[0], at, how.leg);
        // `keep_flag` sets bit 11 (0x141010dc1) off a leg, which the hold's test then decides
        // again
        if how.hold {
            path_flags &= !1;
            g.flags |= 0x800;
        } else {
            g.flags &= !0x800;
        }
        path.measure();
        let path = (2 <= path.points.len()
            && crate::math::fmath::below_or_unordered(EPSILON, path.length))
        .then_some(path);
        if path.is_none() {
            g.flags &= !0x800;
        }
        // FUN_140f98e80: the level and its parents go, its repath time stays
        let repath_time = g.motion.paths.first().map_or(0.0, |l| l.repath_time);
        g.motion.paths.truncate(1);
        let length = path.as_ref().map(|p| p.length);
        let level = move_level(path, path_flags, repath_time, world_time, how.range, target);
        if g.motion.paths.is_empty() {
            g.motion.paths.push(level);
        } else {
            g.motion.paths[0] = level;
        }
        self.stop_all_units(group, false)?;
        let ids: Vec<i32> = self
            .groups
            .get(group)
            .expect("found above")
            .members
            .iter()
            .map(|m| m.id)
            .collect();
        for id in ids {
            // FUN_1415417a0 finds a sleeping member too
            if let Some(ai) = self.object_mut(id).and_then(|(_, u)| u.ai.as_mut()) {
                ai.clear_notifies(world_time)?;
            }
        }
        let owner = self.groups.get(group).expect("found above").owner as usize;
        let min_column = self
            .players
            .get(owner)
            .map_or(0, |p| p.formations.min_column_distance);
        let g = self.groups.get_mut(group).expect("the group found above");
        g.current_target.original_position[0] = point[0];
        g.current_target.original_position[2] = point[1];
        g.current_command = MOVE_COMMAND;
        if g.motion.flags & 2 == 0 && !patrol {
            let column =
                length.is_none_or(|l| crate::math::fmath::below_or_unordered(min_column as f32, l));
            self.update_desired_positions(
                group,
                if column {
                    crate::group::formation::COLUMN
                } else {
                    crate::group::formation::LINE
                },
            )?;
        }
        let g = self.groups.get_mut(group).expect("the group found above");
        g.flags |= 0x100;
        Ok(true)
    }

    /// `checkQueuedOrders` (`FUN_14100d460`) of group `group`: with no queued command a
    /// hibernating group wakes (`+0x13c` 0; flags `0x4c000` when it was 1). Else the last command
    /// replaced none (`+0x13d` 0) and the approved commands run in order (unapproved ones wait):
    /// each takes the run's first command if none was yet and leaves the queue; a move (1), or a
    /// command on a target the world's table does not hold, adds its point as a waypoint while
    /// more than one command is queued ([`UnitGroup::add_waypoint`]); a command on a held target
    /// (3, 5, 0xc, 0x65, 0x69, 0x6a, 0x6e, 0x6f, 0x84, 0x88, 0x2713: `FUN_14100fc50`; 7, 9, 0x68,
    /// 0x7d: the attack, `FUN_14100f7f0`) ends the run; any other is dropped. The run also ends
    /// when the next command differs from its first (the queue's end counts as -1). Then, when
    /// an approved command ran and none ended the run, the group moves to the last command of the
    /// run ([`Turns::command_move_to`] with its range, the target forgotten, `direct`); the
    /// hibernating group wakes as above.
    ///
    /// # Errors
    ///
    /// A command on a target the world holds (`FUN_14100fc50`, `FUN_14100f7f0`, not modelled),
    /// a run whose last command the game frees before its move (it reads a null command: the
    /// run's commands equal an unapproved one behind them), and what the move does not model.
    ///
    /// # Panics
    ///
    /// When the world does not hold `group`: the game's routine gets the group itself, from a
    /// caller that holds it.
    pub fn check_queued_orders(&mut self, group: i32) -> Result<(), GenieError> {
        let g = self
            .groups
            .get_mut(group)
            .expect("the caller holds the group");
        if !g.command_queue.is_empty() {
            g.replaces_queue = 0;
            // a command on a held target would end the run without the move (`bVar2`); those
            // are not modelled (below), so every run that ran an approved command moves
            let (mut first, mut kept, mut ran) = (-1, None, false);
            let mut i = 0;
            loop {
                let g = self.groups.get(group).expect("the group found above");
                let Some(&node) = g.command_queue.get(i) else {
                    break;
                };
                if node.approved == 0 {
                    i += 1;
                    continue;
                }
                ran = true;
                let held = self.world_object(node.target_id).is_some();
                if first == -1 {
                    first = node.command;
                }
                let count = g.command_queue.len();
                let g = self.groups.get_mut(group).expect("the group found above");
                match node.command {
                    c if TARGET_COMMANDS.contains(&c) && held => {
                        return Err(GenieError::unsupported(
                            "a queued command on a target (FUN_14100fc50)",
                        ));
                    }
                    c if ATTACK_COMMANDS.contains(&c) && held => {
                        return Err(GenieError::unsupported(
                            "a queued attack on a target (FUN_14100f7f0)",
                        ));
                    }
                    c if (c == MOVE_COMMAND
                        || TARGET_COMMANDS.contains(&c)
                        || ATTACK_COMMANDS.contains(&c))
                        && 1 < count =>
                    {
                        g.add_waypoint(node.point[0], node.point[2], false);
                    }
                    _ => {}
                }
                let next = g.command_queue.get(i + 1).map_or(-1, |n| n.command);
                let _ = g.command_queue.remove(i);
                let ends = first != -1 && next != first;
                if ends || g.command_queue.is_empty() {
                    kept = Some(node);
                }
                if ends {
                    break;
                }
            }
            if ran {
                let last = kept.ok_or(GenieError::unsupported(
                    "checkQueuedOrders' move to a command it freed (a null command in the game)",
                ))?;
                let _ = self.command_move_to(
                    group,
                    -1,
                    [last.point[0], last.point[2]],
                    MoveTo {
                        range: last.range,
                        forget_target: true,
                        direct: true,
                        ..MoveTo::default()
                    },
                )?;
            }
        }
        let g = self.groups.get_mut(group).expect("the group found above");
        if g.hibernating != 0 {
            if g.hibernating == 1 {
                g.flags |= 0x4_c000;
            }
            g.hibernating = 0;
        }
        Ok(())
    }

    /// `stopAllUnits` (`FUN_14103a0d0`) of group `group`: each member the world holds with an
    /// action (slot 0x530, `actionType`, not -1), or every one with `all`, stops (slot 0x148,
    /// [`Turns::object_stop`]; slot 0x560: its movement stands and its path level resets) and its
    /// own move clears (active 0, path and reason -1, the rest 0).
    ///
    /// # Errors
    ///
    /// A member with an action the slice does not model or a
    /// sleeping one, or what the stop does not model.
    ///
    /// # Panics
    ///
    /// When the world does not hold `group`: the game's routine gets the group itself, from a
    /// caller that holds it.
    pub fn stop_all_units(&mut self, group: i32, all: bool) -> Result<(), GenieError> {
        let ids: Vec<i32> = self
            .groups
            .get(group)
            .expect("the caller holds the group")
            .members
            .iter()
            .map(|m| m.id)
            .collect();
        for (k, id) in ids.into_iter().enumerate() {
            // FUN_1415417a0: a sleeping member too
            let Some(place) = self.place(id) else {
                continue;
            };
            let u = match place {
                Place::Live(p, i) => &self.players[p].objects[i],
                Place::Sleeping(p, i) => &self.players[p].sleeping[i],
            };
            if u.unmodelled_action.is_some() {
                return Err(GenieError::unsupported(
                    "stopAllUnits of a member with an action the slice does not model",
                ));
            }
            // slot 0x530: the top action's type, -1 without one
            if !all && u.top_action_kind().is_none() {
                continue;
            }
            let Place::Live(p, idx) = place else {
                return Err(GenieError::unsupported(
                    "stopAllUnits' stop of a sleeping member (the stop takes the live list)",
                ));
            };
            self.object_stop(p, idx)?;
            if let Some(m) = self.players[p].objects[idx].mover.as_mut() {
                m.stand()?;
            }
            let g = self.groups.get_mut(group).expect("the group found above");
            g.members[k].specific_move = SpecificMove {
                path_id: -1,
                reason: -1,
                ..SpecificMove::default()
            };
        }
        Ok(())
    }
}

/// [`Turns::command_move_to`]'s path flags: a patrol leg sets bit 0 and, when the group's place
/// `from` lies at least [`PATROL_LEG_REACH`] from `to` on the ground (`COMISS`/`JBE`: a NaN
/// too), bit 1; any other move clears bit 0 and sets bit 1.
fn move_path_flags(from: [f32; 3], to: [f32; 3], leg: bool) -> u32 {
    if leg {
        let mut d = crate::math::fmath::sub(from, to);
        d[1] = 0.0;
        let length = crate::math::fmath::sqrt(d[0] * d[0] + 0.0 + d[2] * d[2]);
        if crate::math::fmath::at_most_or_unordered(PATROL_LEG_REACH, length) {
            NEW_PATH_FLAGS | 1 | 2
        } else {
            (NEW_PATH_FLAGS | 1) & !2
        }
    } else {
        NEW_PATH_FLAGS & !1 | 2
    }
}

/// [`Turns::command_move_to`]'s new path level (`FUN_140f98e80`'s reset keeping the repath time,
/// then `FUN_140f9b800`): with a path its points, flags, length, creation time, range, target,
/// waypoint 1 and state 2; waypoint level 2 either way.
fn move_level(
    path: Option<Path>,
    path_flags: u32,
    repath_time: f32,
    world_time: u32,
    range: f32,
    target: i32,
) -> PathData {
    let mut level = PathData {
        repath_time,
        ..PathData::without_path()
    };
    if let Some(p) = path {
        // FUN_140f9b800
        level.points = p.points;
        level.has_path = true;
        level.path_flags = path_flags;
        level.length = p.length;
        level.created = world_time;
        level.range = range;
        level.target = target;
        level.waypoint = 1;
        level.state = 2;
        level.continue_counter = CONTINUE;
    }
    level.level = 2;
    level
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::group::{GroupCommand, Member, UnitGroups};
    use crate::masters::{Master, Masters};
    use crate::turn::{Player, Unit};

    /// Player 1's unit 7 (master 4: type 70, category 1, no action, no unit AI) in group 1 at
    /// the origin, its top speed 1, its motion flags bit 1 set (no formation follows), at world
    /// time 50.
    fn moving_world() -> Turns {
        let mut u = Unit {
            id: 7,
            ..Unit::default()
        };
        u.life.master_id = 4;
        let players = vec![
            Player::default(),
            Player {
                objects: vec![u],
                ..Player::default()
            },
        ];
        let mut t = Turns::new(50, 0, 0, vec![0, 0], players);
        let master = Master {
            id: 4,
            kind: 70,
            formation_category: 1,
            ..Master::default()
        };
        t.masters = vec![Masters::new([master]); 2];
        let mut g = UnitGroup::new(1);
        g.owner = 1;
        g.members = vec![Member {
            id: 7,
            ..Member::default()
        }];
        g.motion.flags = 2;
        g.motion.maximum_velocity_forward = 1.0;
        t.groups = UnitGroups {
            groups: vec![g],
            ..UnitGroups::default()
        };
        t
    }

    fn command(command: i32, target: i32, x: f32, z: f32, approved: u8) -> GroupCommand {
        GroupCommand {
            command,
            target_id: target,
            range: 0.5,
            point: [x, 0.0, z],
            approved,
            disable_retargeting: 0,
            approved_2: 0xffff,
        }
    }

    /// `FUN_141010a80` with `param_9`: the move joins the command queue (`queueOrder`, approved
    /// with `direct`) and nothing moves; a group without a command hibernates for a direct one.
    #[test]
    fn a_queued_move_joins_the_command_queue() {
        let mut t = moving_world();
        let how = MoveTo {
            range: 0.5,
            queued: true,
            direct: true,
            ..MoveTo::default()
        };
        assert!(!t.command_move_to(1, -1, [3.0, 4.0], how).unwrap());
        let g = t.groups.get(1).unwrap();
        assert_eq!(g.command_queue, [command(1, -1, 3.0, 4.0, 1)]);
        assert_eq!((g.hibernating, g.current_command), (1, -1));
        assert!(!g.motion.paths[0].has_path);
    }

    /// `FUN_14100dd10` drops the approved commands and approves the others.
    #[test]
    fn approving_the_queue_drops_the_approved_commands() {
        let mut g = UnitGroup::new(1);
        g.command_queue = vec![command(1, -1, 1.0, 1.0, 1), command(9, 5, 2.0, 2.0, 0)];
        g.approve_queued();
        assert_eq!(g.command_queue, [command(9, 5, 2.0, 2.0, 1)]);
    }

    /// `FUN_14100d460`: an unapproved command waits; of the approved moves each but the last
    /// adds its point as a waypoint while more than one command is queued, and the group moves
    /// through them to the last; a command on a target the world does not hold counts as a
    /// move, one on a held target is not modelled.
    #[test]
    fn the_queued_moves_run_through_their_points() {
        let mut t = moving_world();
        t.groups.get_mut(1).unwrap().command_queue = vec![
            command(9, -1, 9.0, 9.0, 0),
            command(1, -1, 5.0, 5.0, 1),
            command(3, 99, 8.0, 6.0, 1),
        ];
        // 3 differs from the run's first (1): the run ends at the move to (5, 5)
        t.check_queued_orders(1).unwrap();
        let g = t.groups.get(1).unwrap();
        assert_eq!(
            g.command_queue,
            [command(9, -1, 9.0, 9.0, 0), command(3, 99, 8.0, 6.0, 1)]
        );
        assert_eq!(g.motion.paths[0].points, [[0.0; 3], [5.0, 0.0, 5.0]]);
        assert_eq!((g.current_command, g.replaces_queue), (MOVE_COMMAND, 0));
        // two moves: the first is a waypoint, the group goes through it to the second
        let mut t = moving_world();
        t.groups.get_mut(1).unwrap().command_queue =
            vec![command(1, -1, 5.0, 5.0, 1), command(1, -1, 8.0, 8.0, 1)];
        t.check_queued_orders(1).unwrap();
        let g = t.groups.get(1).unwrap();
        assert!(g.command_queue.is_empty() && g.waypoints.is_empty());
        assert_eq!(
            g.motion.paths[0].points,
            [[0.0; 3], [5.0, 0.0, 5.0], [8.0, 0.0, 8.0]]
        );
        // a command on a target the world holds
        let mut t = moving_world();
        t.groups.get_mut(1).unwrap().command_queue = vec![command(3, 7, 1.0, 1.0, 1)];
        assert!(t.check_queued_orders(1).is_err());
        // nothing queued: a hibernating group wakes
        let mut t = moving_world();
        t.groups.get_mut(1).unwrap().hibernating = 1;
        t.check_queued_orders(1).unwrap();
        let g = t.groups.get(1).unwrap();
        assert_eq!((g.hibernating, g.flags & 0x4_c000), (0, 0x4_c000));
    }

    /// A move without `direct` behind queued commands joins them unapproved; the approved ones
    /// go and the queue runs: the group moves to the new point.
    #[test]
    fn a_move_behind_queued_commands_runs_the_queue() {
        let mut t = moving_world();
        t.groups.get_mut(1).unwrap().command_queue = vec![command(1, -1, 5.0, 5.0, 1)];
        let how = MoveTo {
            range: 0.5,
            ..MoveTo::default()
        };
        assert!(t.command_move_to(1, -1, [2.0, 3.0], how).unwrap());
        let g = t.groups.get(1).unwrap();
        assert_eq!(g.command_queue, Vec::<GroupCommand>::new());
        assert_eq!(g.motion.paths[0].points, [[0.0; 3], [2.0, 0.0, 3.0]]);
    }

    /// `param_8` (hold) clears the path's bit 0 and leaves flags bit 11 set; a patrol is read
    /// on the top ancestor's path (`FUN_141046500`), which sets flags `0xc000`; a sleeping
    /// member's unit AI drops its notifies (`FUN_140c5dc10` at the world's time); a NaN length
    /// keeps the path (`COMISS`/`JC`).
    #[test]
    fn a_held_move_keeps_flags_bit_11_and_a_patrol_is_the_top_path_s() {
        let mut t = moving_world();
        let hold = MoveTo {
            hold: true,
            ..MoveTo::default()
        };
        assert!(t.command_move_to(1, -1, [2.0, 3.0], hold).unwrap());
        let g = t.groups.get(1).unwrap();
        assert_eq!(
            (g.flags & 0x800, g.motion.paths[0].path_flags & 1),
            (0x800, 0)
        );
        let mut t = moving_world();
        let mut walked = PathData::without_path();
        walked.has_path = true;
        let mut top = PathData::without_path();
        top.has_path = true;
        top.path_flags = 1;
        t.groups.get_mut(1).unwrap().motion.paths = vec![walked, top];
        let asleep = t.players[1].objects.remove(0);
        t.players[1].sleeping.push(Unit {
            ai: Some(crate::unit_ai::UnitAi::default()),
            ..asleep
        });
        assert!(
            t.command_move_to(1, -1, [2.0, 3.0], MoveTo::default())
                .unwrap()
        );
        let g = t.groups.get(1).unwrap();
        assert_eq!(g.flags & 0xc000, 0xc000);
        let ai = t.players[1].sleeping[0].ai.as_ref().unwrap();
        assert_eq!(ai.last_update_time, 50);
        let mut t = moving_world();
        assert!(
            t.command_move_to(1, -1, [f32::NAN, 3.0], MoveTo::default())
                .unwrap()
        );
        assert!(t.groups.get(1).unwrap().motion.paths[0].has_path);
    }

    /// `param_6` (a patrol's leg): the path loops (bit 0) and goes back and forth (bit 1) from 5
    /// tiles on the ground (`COMISS`/`JBE`: a NaN distance too); flags bit 11 clears, `0xc000`
    /// stays off for a leg on a patrol.
    #[test]
    fn a_patrol_s_leg_loops_and_turns_back_from_five_tiles() {
        let leg = MoveTo {
            leg: true,
            forget_target: true,
            direct: true,
            ..MoveTo::default()
        };
        for (point, flags) in [([4.9, 0.0], 7 & !2), ([3.0, 4.0], 7), ([f32::NAN, 0.0], 7)] {
            let mut t = moving_world();
            t.groups.get_mut(1).unwrap().flags = 0x800;
            assert!(t.command_move_to(1, -1, point, leg).unwrap());
            let g = t.groups.get(1).unwrap();
            assert_eq!(
                (g.motion.paths[0].path_flags, g.flags & 0x800),
                (flags, 0),
                "{point:?}"
            );
        }
        // the group patrols already: a leg leaves flags 0xc000 off
        let mut t = moving_world();
        let mut top = PathData::without_path();
        top.has_path = true;
        top.path_flags = 1;
        t.groups.get_mut(1).unwrap().motion.paths = vec![top];
        assert!(t.command_move_to(1, -1, [8.0, 0.0], leg).unwrap());
        assert_eq!(t.groups.get(1).unwrap().flags & 0xc000, 0);
    }
}
