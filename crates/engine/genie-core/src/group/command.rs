//! A group's orders (FD-036): `BUnitGroup::command` (`FUN_14100efd0`), `commandMoveTo`
//! (`FUN_141010a80`) and `stopAllUnits` (`FUN_14103a0d0`), the HD PDB's names.

use super::SpecificMove;
use crate::movement::PathData;
use crate::order::Place;
use crate::tile_path::Path;
use crate::turn::Turns;
use crate::unit_ai::Unsupported;

/// A group whose top speed is below this does not move (`commandMoveTo`, `DAT_142eb0c60`); nor is
/// a path shorter than it one.
const EPSILON: f32 = crate::image::group::EPSILON;
/// The group's command while it moves (`mCurrentCommand` 1).
pub const MOVE_COMMAND: i32 = 1;
/// A new path's flags (`FUN_14152eea0`'s construction).
const NEW_PATH_FLAGS: u32 = 6;
/// The plans a path level may still try (`FUN_140f9b800`, `FUN_140f98e80`).
const CONTINUE: i32 = 10;

/// What `commandMoveTo` is told besides the target and the point: the range (`param_5`), and its
/// flags `param_6` (a patrol's leg), `param_7` (the group's target goes), `param_8`, `param_9`
/// (queued), `param_10` (at once, though orders are queued) and `param_11`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MoveTo {
    /// `param_5`: the path level's range.
    pub range: f32,
    /// `param_6`.
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

impl Turns {
    /// `allHordeUnits` (`FUN_141008e50`): no member the world holds is of a master above type 69
    /// with a formation category (`+0x8bc`).
    ///
    /// # Errors
    ///
    /// A group the world does not hold, or a member of a master the slice does not have.
    pub fn all_horde_units(&self, group: i32) -> Result<bool, Unsupported> {
        let g = self.groups.get(group).ok_or(Unsupported(
            "allHordeUnits of a group the world does not hold",
        ))?;
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
    /// notes whether the command was queued (`+0x13d`, [`super::UnitGroup::replaces_queue`]) and,
    /// without a task, moves ([`Turns::command_move_to`] with its target forgotten); a move begun
    /// stops the members (`stopAllUnits` with 0, [`Turns::stop_all_units`]), and flags bit 17
    /// clears. True when the move began. A task on a target is not modelled.
    ///
    /// # Errors
    ///
    /// A group the world does not hold, a target, a horde, or what the move does not model.
    ///
    /// # Panics
    ///
    /// Never: the group is the one found first.
    pub fn group_command(
        &mut self,
        group: i32,
        target: i32,
        point: [f32; 2],
        (queued, direct): (bool, bool),
    ) -> Result<bool, Unsupported> {
        let g = self
            .groups
            .get_mut(group)
            .ok_or(Unsupported("a command of a group the world does not hold"))?;
        g.flags &= !0x20;
        if target != -1 {
            return Err(Unsupported(
                "a group's command on a target (its task, slot 0x130)",
            ));
        }
        if self.all_horde_units(group)? {
            return Err(Unsupported(
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

    /// `commandMoveTo` (`FUN_141010a80`) of group `group` to `point` (`target` -1 for none): flags
    /// bits 5 and 12 clear. A queued move, and one behind queued orders without `direct`, go into
    /// the order queue (`queueOrder`, `FUN_14102b640`, not modelled). A group whose last command
    /// was not queued empties its order queue ([`super::UnitGroup::clear_order_queue`]);
    /// `forget_target` clears its target (`mCurrentTarget`: -1 and type -1, its place, range,
    /// radius and retargeting byte 0); a changed target resets the attack plan's entries
    /// (`FUN_140fd76a0` on `+0x3b0`, `mCurrentAttackPlan`, which no save holds, every load
    /// empties and only the attack's plan fills: empty here). Each member's seconds since
    /// attacking are `f32::MAX`. A horde moves as one (`commandHorde`, not modelled). A changed
    /// target, or a patrol's path (the walked level's path flags bit 0, or flags bit 11) when
    /// neither `leg` nor `hold`, sets flags `0xc000`. Below [`EPSILON`] of top speed nothing moves
    /// (false). The path (a new one, flags [`NEW_PATH_FLAGS`], made now) runs from the group's
    /// place through its waypoints (which it empties) to the point at the group's desired
    /// height, its flags bit 0 off and bit 1 on, flags bit 11 off; measured, under two points or
    /// [`EPSILON`] of length it is none. The group's path data resets ([`PathData`]'s
    /// `FUN_140f98e80`) and takes it (`FUN_140f9b800`: range, target, waypoint 1, state 2) as
    /// waypoint level 2 (that level even without a path); the members stop
    /// ([`Turns::stop_all_units`]) and their unit AIs' notifies go at the world's time
    /// (`FUN_140c5dc10`); the target's place is the point, the command [`MOVE_COMMAND`]. With
    /// motion flags bit 1 clear and no patrol the formation follows (`UpdateDesiredPositions`: a
    /// column without a path or past the player's `mMinColumnDistance`, else a line). Flags bit 8
    /// is set. True.
    ///
    /// Not modelled: `leg` (a patrol's), `hold` and `keep_flag` (both clear here).
    ///
    /// # Errors
    ///
    /// A group the world does not hold, a queued move, a horde, or what the members' stop and
    /// the formation do not model.
    ///
    /// # Panics
    ///
    /// Never: the group is the one found first.
    pub fn command_move_to(
        &mut self,
        group: i32,
        target: i32,
        point: [f32; 2],
        how: MoveTo,
    ) -> Result<bool, Unsupported> {
        if how.leg || how.hold || how.keep_flag {
            return Err(Unsupported(
                "commandMoveTo of a patrol's leg or a held formation (param_6, 8, 11)",
            ));
        }
        let world_time = self.world_time;
        let g = self
            .groups
            .get_mut(group)
            .ok_or(Unsupported("a move of a group the world does not hold"))?;
        g.flags &= !0x1020;
        let changed = if target == -1 {
            g.current_target.target != -1
        } else {
            target != g.current_target.target
        };
        if how.queued || (!how.direct && !g.order_queue.is_empty()) {
            return Err(Unsupported(
                "a group's queued move (queueOrder, FUN_14102b640)",
            ));
        }
        if g.replaces_queue != 0 {
            g.clear_order_queue();
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
            return Err(Unsupported("a horde's move (commandHorde, FUN_141010180)"));
        }
        let g = self.groups.get_mut(group).expect("the group found above");
        let patrol = g
            .motion
            .paths
            .first()
            .is_some_and(|l| l.has_path && l.path_flags & 1 != 0)
            || g.flags & 0x800 != 0;
        if (!how.hold && !how.leg && patrol) || changed {
            g.flags |= 0xc000;
        }
        // the top speed below the tolerance (`COMISS`: EPSILON above it)
        if g.motion.maximum_velocity_forward < EPSILON {
            return Ok(false);
        }
        let mut path = Path::default();
        let _ = path.push(g.motion.position[0]);
        for &w in &g.waypoints {
            let _ = path.push(w);
        }
        g.waypoints.clear();
        let _ = path.push([point[0], g.desired_height, point[1]]);
        let path_flags = NEW_PATH_FLAGS & !1 | 2;
        g.flags &= !0x800;
        path.measure();
        let path = (2 <= path.points.len() && EPSILON < path.length).then_some(path);
        if path.is_none() {
            g.flags &= !0x800;
        }
        // FUN_140f98e80: the level and its parents go, its repath time stays
        let repath_time = g.motion.paths.first().map_or(0.0, |l| l.repath_time);
        g.motion.paths.truncate(1);
        let mut level = PathData {
            repath_time,
            ..PathData::without_path()
        };
        let length = path.as_ref().map(|p| p.length);
        if let Some(p) = path {
            // FUN_140f9b800
            level.points = p.points;
            level.has_path = true;
            level.path_flags = path_flags;
            level.length = p.length;
            level.created = world_time;
            level.range = how.range;
            level.target = target;
            level.waypoint = 1;
            level.state = 2;
            level.continue_counter = CONTINUE;
        }
        level.level = 2;
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
            if let Some(Place::Live(p, i)) = self.place(id)
                && let Some(ai) = self.players[p].objects[i].ai.as_mut()
            {
                ai.clear_notifies(world_time)?;
            } else if let Some(Place::Sleeping(..)) = self.place(id) {
                return Err(Unsupported("a group move of a sleeping member"));
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
            let column = length.is_none_or(|l| (min_column as f32) < l);
            self.update_desired_positions(
                group,
                if column {
                    super::formation::COLUMN
                } else {
                    super::formation::LINE
                },
            )?;
        }
        let g = self.groups.get_mut(group).expect("the group found above");
        g.flags |= 0x100;
        Ok(true)
    }

    /// `stopAllUnits` (`FUN_14103a0d0`) of group `group`: each member the world holds with an
    /// action (slot 0x530, `actionType`, not -1), or every one with `all`, stops (slot 0x148,
    /// [`Turns::object_stop`]; slot 0x560: its movement stands and its path level resets) and its
    /// own move clears (active 0, path and reason -1, the rest 0).
    ///
    /// # Errors
    ///
    /// A group the world does not hold, a member with an action the slice does not model or a
    /// sleeping one, or what the stop does not model.
    ///
    /// # Panics
    ///
    /// Never: the group is the one found first.
    pub fn stop_all_units(&mut self, group: i32, all: bool) -> Result<(), Unsupported> {
        let ids: Vec<i32> = self
            .groups
            .get(group)
            .ok_or(Unsupported(
                "stopAllUnits of a group the world does not hold",
            ))?
            .members
            .iter()
            .map(|m| m.id)
            .collect();
        for (k, id) in ids.into_iter().enumerate() {
            let (p, idx) = match self.place(id) {
                None => continue,
                Some(Place::Live(p, idx)) => (p, idx),
                Some(Place::Sleeping(..)) => {
                    return Err(Unsupported("stopAllUnits of a sleeping member"));
                }
            };
            let u = &self.players[p].objects[idx];
            if u.unmodelled_action.is_some() {
                return Err(Unsupported(
                    "stopAllUnits of a member with an action the slice does not model",
                ));
            }
            if !all && u.top_action_kind().is_none() {
                continue;
            }
            self.object_stop(p, idx)?;
            if let Some(m) = self.players[p].objects[idx].mover.as_mut() {
                m.stop()?;
                if !m.paths.is_empty() {
                    m.reset_level();
                }
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
