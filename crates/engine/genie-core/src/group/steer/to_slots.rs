//! The members' moves to their slots, a free member's stop, the line test and a member's
//! steering toward its slot (`FUN_141025fb0`).

use crate::fmath::{Vec3, below_or_unordered};
use crate::group::SpecificMove;
use crate::group::motion::{hold, moved_vector};
use crate::group::steer::{
    ACTION_KIND, ASIDE, ASIDE_TIMER, CLEARANCE, EPSILON, KEEPS_MOVING, MOST_FAILURES, MOVE_TO,
    MOVES,
};
use crate::turn::Turns;
use crate::{GenieError, fmath};

impl Turns {
    /// The members' moves to their slots (`0x14103e8e8`..`0x14103efba`) of group `group`, whose
    /// formation has slots; `differentiators` the differentiator groups' members the update
    /// listed (`DAT_143f15860`, `DAT_143f15a90`).
    ///
    /// First the group's way ahead (`0x14103e8e8`..`0x14103ee8d`): gated by a lobby setting
    /// (`FUN_140c89690` non-null and `FUN_1412ffe10`) and a top path, a group more than 8 tiles
    /// from the top path's end (64, squared) looks for the units in its way (`FUN_1411570c0`) to
    /// push aside (`FUN_140f88750`). That routine returns at once while `DAT_143f01836` is 0, as
    /// nothing writes it; the query, the pathers on a path of the stack and the list before it
    /// change nothing, so the check holds no state.
    ///
    /// Then each member (its count read anew each turn) the world holds: one in a slot of the
    /// formation (by its unit's id, the first) whose own move is active (1) is tested on the line
    /// from where it last moved to (slot 0x520) to the slot's place (`FUN_1411570c0`: the records
    /// grown by its radius times [`CLEARANCE`], the members ignored, its terrain mask's tiles
    /// added): with none in the way it stands (slot 0x560) and its own move ends (the move
    /// zeroed, its path and reason -1); else the move's timer runs down by the update's seconds
    /// (`FUN_1415300e0`), and with the timer and the move's distance both above 0 (`COMISS`
    /// with `JNC`, then `JC`: a NaN keeps it) the member waits this update; else it stands and its
    /// move ends the same way. A member in a slot that does not wait steers to it
    /// ([`Turns::steer_to_slot`], `FUN_141025fb0`). A member in no slot stops (slot 0x5d0) when it
    /// moves (slot 0x5c8), has no `MoveTo` in its action list (slot 0x538 with 1) and its top
    /// action is not in state 4, 5 or 11 (slot 0x2b0). The member's id in `DAT_143cc29a8` is the
    /// log's.
    ///
    /// # Errors
    ///
    /// What the members' tests and steering do not model.
    ///
    /// # Panics
    ///
    /// When the world does not hold `group`: the group update passes its own.
    pub fn members_to_slots(
        &mut self,
        group: i32,
        differentiators: &[i32],
    ) -> Result<(), GenieError> {
        let mut i = 0;
        loop {
            let g = self.groups.get(group).expect("the caller holds the group");
            let Some(m) = g.members.get(i).copied() else {
                break;
            };
            let index = i;
            i += 1;
            // members are units: the world's table holds ghosts only at negative ids
            let Some((_, u)) = self.object(m.id) else {
                continue;
            };
            let slot = g
                .current_formation
                .as_ref()
                .and_then(|f| f.slots.iter().position(|s| s.unit_id == m.id));
            let Some(slot) = slot else {
                self.stop_free_member(m.id)?;
                continue;
            };
            if m.specific_move.active == 1 {
                let to = g.current_formation.as_ref().expect("found above").slots[slot]
                    .transform_position;
                let members: Vec<i32> = g.members.iter().map(|m| m.id).collect();
                let clear = self.line_clear(u, moved_vector(u), to, CLEARANCE, &members)?;
                let dt = self.dt_seconds;
                let g = self.groups.get_mut(group).expect("held");
                let own = &mut g.members[index].specific_move;
                let ends = if clear {
                    true
                } else {
                    own.timer -= dt;
                    !below_or_unordered(0.0, own.timer) || !below_or_unordered(0.0, own.distance)
                };
                if !ends {
                    continue;
                }
                if let Some((_, u)) = self.object_mut(m.id)
                    && let Some(mover) = u.mover.as_mut()
                {
                    mover.stand()?;
                }
                self.groups.get_mut(group).expect("held").members[index].specific_move =
                    SpecificMove {
                        path_id: -1,
                        reason: -1,
                        ..SpecificMove::default()
                    };
            }
            self.steer_to_slot(group, index, slot, differentiators)?;
        }
        Ok(())
    }

    /// A member in no slot (`0x14103ef2e`): it stops (slot 0x5d0) when its action list holds no
    /// `MoveTo` (slot 0x538 with 1), its top action is not in state 4, 5 or 11 (slot 0x2b0) and
    /// it moves (slot 0x5c8).
    pub(super) fn stop_free_member(&mut self, id: i32) -> Result<(), GenieError> {
        let Some((p, u)) = self.object(id) else {
            return Ok(());
        };
        let (keeps_moving, _) = self.action_list(p, u)?;
        if keeps_moving
            || !u
                .mover
                .as_ref()
                .is_some_and(crate::movement::Mover::is_moving)
        {
            return Ok(());
        }
        if let Some((_, u)) = self.object_mut(id)
            && let Some(mover) = u.mover.as_mut()
        {
            mover.stop()?;
        }
        Ok(())
    }

    /// What the members' stops read of a unit's actions: whether its action list holds a
    /// `MoveTo` (slot 0x538 with 1, `FUN_14106d0c0`) or its top action is in state 4, 5 or 11
    /// (slot 0x2b0, `RGE_Action_Object::actionState`), and that state (0 without an action, and
    /// for an object below type 40). The slice holds the top action alone, as the list's.
    ///
    /// # Errors
    ///
    /// An action the slice does not model, and an object of a master the slice does not have.
    pub(super) fn action_list(
        &self,
        p: usize,
        u: &crate::turn::Unit,
    ) -> Result<(bool, u8), GenieError> {
        if self.master_of(p, u.life.master_id)?.kind < ACTION_KIND {
            return Ok((false, 0));
        }
        if let Some(e) = u.unmodelled_action.clone() {
            return Err(e);
        }
        let top = u.top_action();
        let state = top.map_or(0, |a| a.state);
        let move_to = top.is_some_and(|a| a.kind == MOVE_TO);
        Ok((move_to || KEEPS_MOVING.contains(&state), state))
    }

    /// `FUN_1411570c0` for unit `u` on the line `from` -> `to` (x, height, y; held inside the
    /// map, [`hold`]): no record of its terrain mask's tiles (`FUN_141052b90` with its
    /// restriction's mask) or of an object but the `ignore`d ones (itself alone for none) meets
    /// it, each grown by `grow` times its master's radius on x.
    ///
    /// # Errors
    ///
    /// A record whose place the slice does not know near the line.
    ///
    /// # Panics
    ///
    /// For a unit without movement state: a group's members move ([`MOVES`]).
    pub(super) fn line_clear(
        &self,
        u: &crate::turn::Unit,
        from: Vec3,
        to: Vec3,
        grow: f32,
        ignore: &[i32],
    ) -> Result<bool, GenieError> {
        let mover = u.mover.as_ref().expect(MOVES);
        let own = [u.id];
        let ignore = if ignore.is_empty() { &own[..] } else { ignore };
        // FUN_1411570c0 sets the manager's ignore list to it (FUN_1410562e0)
        let map = self.spatial.map;
        let hits = self.spatial.line_query(
            hold(from, map),
            hold(to, map),
            grow * mover.stats.radius[0],
            Some(mover.stats.restriction_mask),
            ignore,
        )?;
        Ok(hits.is_empty())
    }

    /// `FUN_141025fb0`: member `index` of group `group` steers toward formation slot `slot`
    /// (its place `+0x1c`, time `+0x28` and state `+0x2c`); `differentiators` the update's list
    /// of the differentiator groups' members. Nothing for a unit the world does not hold or
    /// without a unit AI. The gap is the ground distance from where the unit last moved to
    /// (slot 0x520) to the slot's place.
    ///
    /// - In place (the gap at most 1e-7, `COMISS` with `JC`: not a NaN) and in slot state 3:
    ///   the member's failures (`+0x1c` of its own move) clear, and the end.
    /// - More than 10 failures: the member's own move becomes a move aside
    ///   ([`Turns::set_specific_move`], reason 2, its place, [`ASIDE_TIMER`]), it stops (slot
    ///   0x5d0) and the group's flags take bit 9; the end without the goal.
    /// - A member that is not free ([`Turns::member_idle`]) stops as one in no slot does, its
    ///   failures clear and a slot in state 1 or 2 takes state 3; the end without the goal. A
    ///   free one in a slot of state 3 whose group moves or waits for a path (`FUN_141046600`,
    ///   `FUN_141046640`) has the slot take state 0.
    /// - A slot whose time is past (unsigned, below the world's time): state 1 becomes 2 and the
    ///   member stops (slot 0x5d0). In state 2, a group that does not move tries the slot: the
    ///   long pather (`FUN_140f9eb70`, from the unit's tile, range 0) for a unit without a
    ///   footprint and a height (`FUN_141194060` with 1), else the short pather (`FUN_140f9e540`,
    ///   range 0, the members ignored); a way found (the short pather's result not 3 or 4) puts
    ///   the unit on the slot ([`Turns::place_member`], slot 0x58) facing the group's forward
    ///   (slot 0x568 with the group's up). Either way the slot takes state 3. In state 2 or 3 the
    ///   member shows its standing sprite (slot 0x660, [`Turns::show_standing`]), faces the
    ///   group's forward again when it was put, its failures clear, and the end without the
    ///   goal.
    /// - Then the line to the slot ([`Turns::line_clear`] with [`CLEARANCE`], the update's list
    ///   ignored): one that meets a record is the way round ([`Turns::way_round`]). A clear line steers
    ///   the unit: the direction to the slot (slot 0x610), the turn from its forward (slot 0x618
    ///   with dt, [`crate::movement::Mover::turn_dt`]) and the yaw (slot 0x5c0), where its speed
    ///   takes it this update (slot 0x638) and the slot's state from that
    ///   ([`passes_waypoint`], `FUN_141046130`). A slot in state 2 stops the unit (slot 0x5d0),
    ///   which shows its standing sprite when paused (slot 0x600) or when neither the group's
    ///   top path (`FUN_141046500`) is a patrol (flags bit 0) nor its walked level holds points.
    ///   Else by the gap and the radius (master `+0x5d8`; `COMISS` with `SETNC`, then `JC`: a NaN
    ///   is neither): between the radius and five radii, the unit's top speed (slot 0x208) and
    ///   its walking sprite (slot 0x658, [`Turns::show_walking`]); within the radius, the group's
    ///   top speed (`+0xbc`) and the walking sprite; farther, the group's flags take bit 9, and
    ///   from five tiles on (or a NaN) the top speed and the walking sprite (bit 9 again), nearer
    ///   the top speed times 1.2 from three tiles on (or a NaN), below three
    ///   `speed + ((speed × 1.2 − speed) × gap) / 3`, with its running sprite (slot 0x650,
    ///   [`Turns::show_running`]) or else its walking one. The speed is slot 0x598's
    ///   ([`crate::movement::Mover::set_speed`]).
    /// - The goal: the unit AI's `state_position` (`+0x1ac`, `+0x1b0`) takes the slot's x and y.
    ///
    /// The logs (`FUN_140a7f7e0`, `FUN_140f3ee00`) and the slots whose answers only they read
    /// (0x5c8, 0x538, 0x2b0, 0x530, 0x208, 0x590, 0x1f8) hold no state.
    ///
    /// # Errors
    ///
    /// What the steps and the way round do not model.
    ///
    /// # Panics
    ///
    /// When the world does not hold `group`, or `index` or `slot` is outside it: the members'
    /// moves pass their own.
    ///
    /// [`passes_waypoint`]: crate::group::motion::passes_waypoint
    pub fn steer_to_slot(
        &mut self,
        group: i32,
        index: usize,
        slot: usize,
        differentiators: &[i32],
    ) -> Result<(), GenieError> {
        let g = self.groups.get(group).expect("the caller holds the group");
        let id = g.members[index].id;
        let Some((p, u)) = self.object(id) else {
            return Ok(());
        };
        if u.ai.is_none() {
            return Ok(());
        }
        let s = g.current_formation.as_ref().expect("a formation").slots[slot];
        let to = s.transform_position;
        let late = s.time < self.world_time;
        let at = moved_vector(u);
        let mut d = fmath::sub(to, at);
        d[1] = 0.0;
        let gap = fmath::sqrt(d[0] * d[0] + 0.0 + d[2] * d[2]);
        if !below_or_unordered(EPSILON, gap) && s.state == 3 {
            self.groups.get_mut(group).expect("held").members[index]
                .specific_move
                .consecutive_move_failures = 0;
            self.set_state_position(id, to);
            return Ok(());
        }
        if MOST_FAILURES < g.members[index].specific_move.consecutive_move_failures {
            let _ = self.set_specific_move(group, id, (-1, ASIDE), (at, ASIDE_TIMER, 0.0));
            self.stop_member(id)?;
            self.groups.get_mut(group).expect("held").flags |= 0x200;
            return Ok(());
        }
        if !self.member_idle(id)? {
            let (keeps_moving, _) = self.action_list(p, u)?;
            if !keeps_moving
                && u.mover
                    .as_ref()
                    .is_some_and(crate::movement::Mover::is_moving)
            {
                self.stop_member(id)?;
            }
            let g = self.groups.get_mut(group).expect("held");
            g.members[index].specific_move.consecutive_move_failures = 0;
            let state = &mut g.current_formation.as_mut().expect("a formation").slots[slot].state;
            if matches!(*state, 1 | 2) {
                *state = 3;
            }
            return Ok(());
        }
        let g = self.groups.get_mut(group).expect("held");
        let moving = g.motion.moving();
        let waits = g.motion.waits_to_move();
        let state = &mut g.current_formation.as_mut().expect("a formation").slots[slot].state;
        if *state == 3 && (moving || waits) {
            *state = 0;
        }
        if late && self.slot_due(group, index, slot)? {
            return Ok(());
        }
        let Some((_, u)) = self.object(id) else {
            return Ok(());
        };
        if !self.line_clear(u, moved_vector(u), to, CLEARANCE, differentiators)? {
            return self.way_round(group, index, slot, gap, differentiators);
        }
        self.steer_member(group, slot, id, gap)?;
        self.set_state_position(id, to);
        Ok(())
    }
}
