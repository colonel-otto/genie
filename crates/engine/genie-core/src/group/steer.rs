//! The members' moves to their slots (FD-036): the part of `BUnitGroup::update`
//! (`FUN_14103d910`, `0x14103e8e8`..`0x14103efba`) that brings each member of a formation to its
//! slot, with the member's steering (`FUN_141025fb0`) and the setter of a member's own move
//! (`FUN_141039f60`).

use crate::fmath::{self, Vec3, at_most_or_unordered, below_or_unordered};
use crate::group::SpecificMove;
use crate::group::motion::{hold, moved_vector, passes_waypoint};
use crate::turn::Turns;
use crate::unit_ai::Unsupported;

/// A member this near its slot on the ground is in place (`DAT_142eb0c60`, `0x14102629e`).
const EPSILON: f32 = crate::image::group::EPSILON;
/// The members' obstruction tests grow the records by the member's radius times this
/// (`DAT_142e7f384`).
// role pending: group::CLEARANCE, 0x142e7f384, f32, read into XMM7 at 0x14103e5fd (the member's
// test at 0x14103f143) and at 0x1410267f2 (FUN_141025fb0's test)
const CLEARANCE: f32 = f32::from_bits(0x3f00_0000);
/// A member whose moves to its slot failed more than this many times in a row is moved aside
/// (`FUN_141025fb0`'s `0xa`).
const MOST_FAILURES: i32 = 10;
/// The timer of the move aside after too many failures (`DAT_142eb5690`).
// role pending: group::ASIDE_TIMER, 0x142eb5690, f32, the MOVSS at 0x14102632d
const ASIDE_TIMER: f32 = f32::from_bits(0x3dcc_cccd);
/// A member's own move for being moved aside (`mReason`).
const ASIDE: i32 = 2;
/// A member within this many radii of its slot, or this far, walks at its top speed
/// (`DAT_14330ebb8`, loaded once at `0x1410279d1` for both).
// role pending: group::SLOT_REACH, 0x14330ebb8, f32, the MOVSS at 0x1410279d1
const SLOT_REACH: f32 = f32::from_bits(0x40a0_0000);
/// Nearer its slot than [`SLOT_REACH`], a member catches up at its top speed times
/// [`CATCH_UP`], less the nearer it is within this (`DAT_142eb1c98`).
// role pending: group::CATCH_UP_SPAN, 0x142eb1c98, f32, the MOVSS at 0x141027b29
const CATCH_UP_SPAN: f32 = f32::from_bits(0x4040_0000);
/// The catching up's share of the top speed (`DAT_142ed66b4`).
// role pending: group::CATCH_UP, 0x142ed66b4, f32, the MULSS at 0x141027b3d and at 0x141027b83
const CATCH_UP: f32 = f32::from_bits(0x3f99_999a);
/// A moving object whose top speed is 0 and whose radius on either axis is above this links its
/// tiles at a new place (`FUN_14108c320`, `DAT_142e7f384`).
// role pending: group::LINKED_RADIUS, 0x142e7f384, f32, the MOVSS at 0x14108c5af
const LINKED_RADIUS: f32 = f32::from_bits(0x3f00_0000);
/// The action types and states the stop of a member without a slot passes over: a `MoveTo`
/// in the action list (slot 0x538 with 1), and a top action in state 4, 5 or 11 (slot 0x2b0).
const MOVE_TO: i16 = crate::move_to::MOVE_TO;
const KEEPS_MOVING: [u8; 3] = [4, 5, 11];
/// The types of action objects (`RGE_Action_Object`) and of buildings (`TRIBE_Building_Object`).
const ACTION_KIND: u8 = 40;
const BUILDING_KIND: u8 = 80;
/// A member's way to its slot meets a record (`FUN_141025fb0` from `0x14102683b`): the way
/// round takes a world path record (`FUN_14152eea0`), whose id the member's own move keeps
/// (`FUN_141039f60` at `0x14102778f`) and the slice does not hold; the pathers on it
/// (`FUN_140f9e540`, `FUN_140f7c530`), the position test (slot 0x270), `FUN_140a847c0` and
/// slot 0x540 follow.
const BLOCKED_WAY: Unsupported = Unsupported(
    "a member's way to its slot blocked (FUN_141025fb0 at 0x14102683b: the world's path records' ids)",
);

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
    pub(crate) fn members_to_slots(
        &mut self,
        group: i32,
        differentiators: &[i32],
    ) -> Result<(), Unsupported> {
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
    fn stop_free_member(&mut self, id: i32) -> Result<(), Unsupported> {
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
    fn action_list(&self, p: usize, u: &crate::turn::Unit) -> Result<(bool, u8), Unsupported> {
        if self.master_of(p, u.life.master_id)?.kind < ACTION_KIND {
            return Ok((false, 0));
        }
        if let Some(e) = u.unmodelled_action {
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
    /// A record whose place the slice does not know near the line, and a unit without movement
    /// state.
    fn line_clear(
        &self,
        u: &crate::turn::Unit,
        from: Vec3,
        to: Vec3,
        grow: f32,
        ignore: &[i32],
    ) -> Result<bool, Unsupported> {
        let mover = u
            .mover
            .as_ref()
            .ok_or(Unsupported("a member without movement state"))?;
        let own = [u.id];
        let ignore = if ignore.is_empty() { &own[..] } else { ignore };
        let map = self.spatial.map;
        let hits = self.spatial.line_hits(
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
    ///   ignored): one that meets a record is the way round ([`BLOCKED_WAY`]). A clear line steers
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
    /// The way round a record ([`BLOCKED_WAY`]) and what the steps do not model.
    ///
    /// # Panics
    ///
    /// When the world does not hold `group`, or `index` or `slot` is outside it: the members'
    /// moves pass their own.
    pub(crate) fn steer_to_slot(
        &mut self,
        group: i32,
        index: usize,
        slot: usize,
        differentiators: &[i32],
    ) -> Result<(), Unsupported> {
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
            return Err(BLOCKED_WAY);
        }
        self.steer_member(group, slot, id, gap)?;
        self.set_state_position(id, to);
        Ok(())
    }

    /// `FUN_141025fb0` for a slot whose time is past (`0x1410263e4`..`0x1410267c5`): true when
    /// it ends the routine (slot state 2 or 3). See [`Turns::steer_to_slot`].
    fn slot_due(&mut self, group: i32, index: usize, slot: usize) -> Result<bool, Unsupported> {
        let g = self.groups.get(group).expect("held");
        let id = g.members[index].id;
        let s = g.current_formation.as_ref().expect("a formation").slots[slot];
        if s.state == 1 {
            self.set_slot_state(group, slot, 2);
            self.stop_member(id)?;
        }
        let g = self.groups.get(group).expect("held");
        let state = g.current_formation.as_ref().expect("a formation").slots[slot].state;
        let mut placed = false;
        match state {
            2 => {
                if !g.motion.moving() {
                    placed = self.try_slot(group, id, s.transform_position)?;
                }
                self.set_slot_state(group, slot, 3);
            }
            3 => {}
            _ => return Ok(false),
        }
        self.show_standing(id)?;
        if placed {
            self.face_group(group, id);
        }
        self.groups.get_mut(group).expect("held").members[index]
            .specific_move
            .consecutive_move_failures = 0;
        Ok(true)
    }

    /// A due slot's try (`0x141026420`..`0x14102677c`): whether member `id` of group `group`
    /// has a way to `to` (the long pather's, or the short pather's for a unit with a footprint
    /// and a height) and so stands on it, facing the group's forward (slot 0x568).
    ///
    /// # Errors
    ///
    /// What the pathers and the position setter do not model.
    fn try_slot(&mut self, group: i32, id: i32, to: Vec3) -> Result<bool, Unsupported> {
        let (p, u) = self.object(id).expect("the member is held");
        let mover = u
            .mover
            .as_ref()
            .ok_or(Unsupported("a member without movement state"))?;
        let g = self.groups.get(group).expect("held");
        let computer = self.players.get(p).is_some_and(|q| q.computer);
        let solid = mover.stats.radius.iter().all(|&r| 0.0 < r);
        let found = if solid {
            let members: Vec<i32> = g.members.iter().map(|m| m.id).collect();
            let at = moved_vector(u);
            let walker = mover.walker(id);
            let mut plan = crate::movement::Planning {
                pathing: &mut self.pathing,
                now: self.world_time,
                computer,
                object: self.streams.get(crate::rng::Stream::Object),
                construction_rate: 0.0,
                groups: &self.groups,
            };
            let mut path = crate::tile_path::Path::default();
            let r = crate::movement::short_path_for(
                walker,
                [at[0], at[2]],
                &self.spatial,
                &mut plan,
                (to, 0.0, -1),
                &members,
                &mut path,
            )?;
            !matches!(r, 3 | 4)
        } else {
            let walker = crate::movement::LongWalker {
                id,
                position: u.position,
                radius: [mover.stats.radius[0], mover.stats.radius[1]],
                mask: mover.stats.restriction_mask,
                moves: mover.stats.speed != 0.0
                    || u.life.master_id == crate::movement::LONG_PATH_MOVER,
                ignore_units: &mover.ignore_units,
                group_units: self.groups.group_units(u.group.id),
            };
            crate::movement::long_path_for(
                &walker,
                [u.position[0], u.position[1]],
                &self.spatial,
                computer,
                (to, 0.0),
                None,
            )?
            .0
        };
        if found {
            self.place_member(id, to)?;
            self.face_group(group, id);
        }
        Ok(found)
    }

    /// The steering of a member on a clear line to its slot (`0x1410277eb`..`0x141027bff`):
    /// see [`Turns::steer_to_slot`]. `gap` is the ground distance to the slot.
    fn steer_member(
        &mut self,
        group: i32,
        slot: usize,
        id: i32,
        gap: f32,
    ) -> Result<(), Unsupported> {
        let g = self.groups.get(group).expect("held");
        let to = g.current_formation.as_ref().expect("a formation").slots[slot].transform_position;
        let dt = self.dt_seconds;
        let (_, u) = self.object_mut(id).expect("the member is held");
        let facets = u.sprite.map(|s| crate::facing::SpriteFacets {
            count: s.graphic.facets,
            flag_1: s.graphic.flags & 2 != 0,
        });
        let at = moved_vector(u);
        let mover = u
            .mover
            .as_mut()
            .ok_or(Unsupported("a member without movement state"))?;
        let direction = mover.direction_to(at, to);
        let (turn, _) = mover.turn_toward(mover.forward, direction, mover.turn_dt(dt));
        let facets = facets.or_else(|| mover.facets());
        mover.yaw(turn, facets)?;
        let next = mover.projected(direction, dt);
        let state = i32::from(passes_waypoint(to, at, next));
        let radius = mover.stats.radius[0];
        let speed = mover.stats.speed;
        self.set_slot_state(group, slot, state);
        let within = !below_or_unordered(radius, gap);
        let near = !within && !below_or_unordered(radius * SLOT_REACH, gap);
        if state == 2 {
            self.stop_member(id)?;
            let (_, u) = self.object(id).expect("held");
            let paused = u.mover.as_ref().is_some_and(crate::movement::Mover::paused);
            let g = self.groups.get(group).expect("held");
            let patrol = crate::group::formation::top_path(&g.motion.paths)
                .is_some_and(|p| p.path_flags & 1 != 0);
            let walks = g
                .motion
                .paths
                .first()
                .is_some_and(|l| l.has_path && !l.points.is_empty());
            if paused || (!patrol && !walks) {
                self.show_standing(id)?;
            }
        } else if near {
            self.set_member_speed(id, speed);
            let _ = self.show_walking(id)?;
        } else if within {
            let top = self
                .groups
                .get(group)
                .expect("held")
                .motion
                .maximum_velocity_forward;
            self.set_member_speed(id, top);
            let _ = self.show_walking(id)?;
        } else {
            self.groups.get_mut(group).expect("held").flags |= 0x200;
            if at_most_or_unordered(SLOT_REACH, gap) {
                self.set_member_speed(id, speed);
                let _ = self.show_walking(id)?;
                self.groups.get_mut(group).expect("held").flags |= 0x200;
            } else {
                let v = if at_most_or_unordered(CATCH_UP_SPAN, gap) {
                    speed * CATCH_UP
                } else {
                    speed + ((speed * CATCH_UP - speed) * gap) / CATCH_UP_SPAN
                };
                self.set_member_speed(id, v);
                if !self.show_running(id)? {
                    let _ = self.show_walking(id)?;
                }
            }
        }
        Ok(())
    }

    /// The unit AI's `state_position` (`+0x1ac`, `+0x1b0`) of unit `id` takes the slot place
    /// `to`'s x and y (`LAB_141027c88`).
    fn set_state_position(&mut self, id: i32, to: Vec3) {
        if let Some((_, u)) = self.object_mut(id)
            && let Some(ai) = u.ai.as_mut()
        {
            ai.state_position = [to[0], to[2]];
        }
    }

    /// Slot `slot` of group `group`'s formation takes state `state`.
    fn set_slot_state(&mut self, group: i32, slot: usize, state: i32) {
        let g = self.groups.get_mut(group).expect("held");
        g.current_formation.as_mut().expect("a formation").slots[slot].state = state;
    }

    /// Slot 0x5d0 of member `id`: it stops ([`crate::movement::Mover::stop`]).
    fn stop_member(&mut self, id: i32) -> Result<(), Unsupported> {
        if let Some((_, u)) = self.object_mut(id)
            && let Some(mover) = u.mover.as_mut()
        {
            mover.stop()?;
        }
        Ok(())
    }

    /// Slot 0x598 of member `id` ([`crate::movement::Mover::set_speed`]).
    fn set_member_speed(&mut self, id: i32, speed: f32) {
        if let Some((_, u)) = self.object_mut(id)
            && let Some(mover) = u.mover.as_mut()
        {
            mover.set_speed(speed);
        }
    }

    /// Slot 0x568 of member `id` with group `group`'s forward and up vectors (`+0x14`,
    /// `+0x20`, [`crate::movement::Mover::set_orientation`]).
    fn face_group(&mut self, group: i32, id: i32) {
        let g = self.groups.get(group).expect("held");
        let [_, forward, up, _] = g.motion.position;
        if let Some((_, u)) = self.object_mut(id)
            && let Some(mover) = u.mover.as_mut()
        {
            mover.set_orientation(forward, up);
        }
    }

    /// Slot 0x58 of member `id` (`FUN_14108c320`) with the place `to` (x, height, y): the
    /// position setter (`FUN_141161950`: held inside the map for a unit that moves,
    /// [`crate::movement::Mover::set_position`]; the ground's height by its fly mode, the place
    /// and the time of its last move, the world's places and the planners' records follow, as
    /// the motion step's). Then, for an object of an id from 0 below type 80 whose top speed
    /// (slot 0x208) is 0 (`UCOMISS` with `JP`: not a NaN) and whose radius on x or y is above
    /// 0.5 (`COMISS` with `JA`, then `JBE`), its tiles link to its new place (`FUN_140f73900`,
    /// [`crate::visibility::Visibility::link_building`]). The moved records of the objects
    /// fastened to a combat object (`+0x8b0`..`+0x8f0`), which the slice does not hold, are a
    /// unit's none.
    ///
    /// # Errors
    ///
    /// An object of a master the slice does not have.
    fn place_member(&mut self, id: i32, [x, h, y]: Vec3) -> Result<(), Unsupported> {
        let Some(place) = self.place(id) else {
            return Ok(());
        };
        let (p, master_id) = {
            let (p, u) = self.object(id).expect("placed");
            (p, u.life.master_id)
        };
        let kind = self.master_of(p, master_id)?.kind;
        let u = match place {
            crate::order::Place::Live(p, i) => &mut self.players[p].objects[i],
            crate::order::Place::Sleeping(p, i) => &mut self.players[p].sleeping[i],
        };
        let Some(mover) = u.mover.as_mut() else {
            return Ok(());
        };
        let map = self.spatial.map;
        let at = mover.set_position([x, y, h], map);
        let at = mover.grounded(at, self.spatial.terrain.as_ref());
        mover.mark_move(at, self.world_time);
        let moving = mover.is_moving();
        let (speed, radius) = (mover.stats.speed, mover.stats.radius);
        u.position = at;
        self.spatial.set_position(id, at);
        if let Some(placed) = self.spatial.get(id) {
            self.pathing.moved(id, placed, moving);
        }
        if 0 <= id
            && kind < BUILDING_KIND
            && speed == 0.0
            && (LINKED_RADIUS < radius[0] || LINKED_RADIUS < radius[1])
            && let Some(v) = self.spatial.sight.as_mut()
        {
            let _ = v.link_building(at, [radius[0], radius[1]]);
        }
        Ok(())
    }

    /// `FUN_141039f60`: member `id`'s own move in group `group`: with a reason other than -1 it
    /// is active (1) with path `path`, reason `reason`, the goal, timer and distance of `goal`
    /// and the place it last moved to (slot 0x520) as its last position, its failures (`+0x1c`)
    /// left; with -1 it ends (zeroed, its path and reason -1). False (0) for a unit that is no
    /// member or that the world does not hold.
    pub(crate) fn set_specific_move(
        &mut self,
        group: i32,
        id: i32,
        (path, reason): (i32, i32),
        (goal, timer, distance): (Vec3, f32, f32),
    ) -> bool {
        let Some(index) = self
            .groups
            .get(group)
            .and_then(|g| g.members.iter().position(|m| m.id == id))
        else {
            return false;
        };
        let Some((_, u)) = self.object(id) else {
            return false;
        };
        let last = moved_vector(u);
        let own = &mut self.groups.get_mut(group).expect("held").members[index].specific_move;
        *own = if reason == -1 {
            SpecificMove {
                path_id: -1,
                reason: -1,
                ..SpecificMove::default()
            }
        } else {
            SpecificMove {
                active: 1,
                path_id: path,
                reason,
                goal,
                timer,
                distance,
                last_position: last,
                ..*own
            }
        };
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::group::{Formation, Member, Slot, UnitGroup, UnitGroups};
    use crate::masters::{Master, Masters};
    use crate::spatial::{Obstruction, Placed, Spatial, Tiles};
    use crate::turn::{Player, Unit};
    use crate::unit_ai::UnitAi;

    /// The update's seconds (69 ms).
    const DT: f32 = 0.069;

    /// A unit of master 4 at (`x`, `y`), where it last moved to, with a free unit AI and the
    /// hunt tests' walker (top speed 0.8, radius 0.2, turn rate the largest), whose walking,
    /// running and standing speeds are 0.9, 1.5 and 0.
    fn member(id: i32, x: f32, y: f32) -> Unit {
        let mut mover = crate::hunt::tests::walker();
        mover.moved_at = [x, y, 0.0];
        mover.stats.walk_speed = 0.9;
        mover.stats.run_speed = 1.5;
        mover.stats.stand_speed = 0.0;
        let mut u = Unit {
            id,
            position: [x, y, 0.0],
            ai: Some(UnitAi {
                current_order: -1,
                current_action: -1,
                current_target: -1,
                ..UnitAi::default()
            }),
            mover: Some(mover),
            ..Unit::default()
        };
        u.life.master_id = 4;
        u.life.state = 2;
        u.life.hp = 50.0;
        u.group.id = 1;
        u
    }

    /// A slot of `unit` at (`x`, `y`) on the map, made at the world's time 100.
    fn slot(unit: i32, x: f32, y: f32) -> Slot {
        Slot {
            unit_id: unit,
            transform_position: [x, 0.0, y],
            time: 100,
            ..Slot::default()
        }
    }

    /// Player 1's units 10 at (5, 5) and 11 at (6, 5) in group 1 (its top speed 0.9), whose
    /// formation's slots are at (10, 5) and (6, 9); master 4 a combat unit of class 6 with
    /// standing sprite 6, walking sprite 7 and running sprite 8 (the slice's graphics hold none,
    /// nor the units' sprites); a 32 by 32
    /// map of the terrain class bit 2 (in the walker's mask) at the world's time 100 and an update of 69 ms.
    fn world() -> Turns {
        let players = vec![
            Player::default(),
            Player {
                objects: vec![member(10, 5.0, 5.0), member(11, 6.0, 5.0)],
                ..Player::default()
            },
        ];
        let mut t = Turns::new(0, 0, 0, vec![0; 2], players);
        t.world_time = 100;
        t.dt_seconds = DT;
        let master = Master {
            id: 4,
            kind: 70,
            class: 6,
            hit_points: 60,
            radius: [0.2, 0.2],
            standing_sprite: 6,
            move_sprite: 7,
            run_sprite: 8,
            formation_category: 1,
            ..Master::default()
        };
        t.masters = vec![Masters::new([master]); 2];
        t.spatial = Spatial::with_tiles([32, 32], Tiles::new(32, 32, vec![2; 32 * 32]));
        let mut g = UnitGroup::new(1);
        g.owner = 1;
        g.flags = 0;
        g.motion.maximum_velocity_forward = 0.9;
        g.members = [10, 11]
            .map(|id| Member {
                id,
                ..Member::default()
            })
            .to_vec();
        g.current_formation = Some(Formation {
            slots: vec![slot(10, 10.0, 5.0), slot(11, 6.0, 9.0)],
            ..Formation::default()
        });
        t.groups = UnitGroups {
            groups: vec![g],
            ..UnitGroups::default()
        };
        t
    }

    /// An obstruction of player 2 at (`x`, `y`).
    fn post(x: f32, y: f32) -> Placed {
        Placed {
            position: [x, y, 0.0],
            radius: [0.2, 0.2],
            class: 4,
            player: 2,
            master: 0,
            obstruction_type: 0,
            buildable_on: 0,
            tall: true,
            speed: 0.0,
            unknown: None,
            obstruction: Some(Obstruction { kind: 5, class: 2 }),
        }
    }

    fn unit(t: &Turns, id: i32) -> &Unit {
        t.object(id).unwrap().1
    }

    fn mover(t: &Turns, id: i32) -> &crate::movement::Mover {
        unit(t, id).mover.as_ref().unwrap()
    }

    fn state(t: &Turns, slot: usize) -> i32 {
        t.groups
            .get(1)
            .unwrap()
            .current_formation
            .as_ref()
            .unwrap()
            .slots[slot]
            .state
    }

    /// `FUN_141039f60`: a reason other than -1 makes the member's move active with its goal,
    /// timer, distance and the place it last moved to, its failures left; -1 ends it; a unit
    /// that is no member, or that the world does not hold, is refused.
    #[test]
    fn a_member_s_own_move_is_set_or_ended() {
        let mut t = world();
        t.groups.get_mut(1).unwrap().members[0]
            .specific_move
            .consecutive_move_failures = 4;
        assert!(t.set_specific_move(1, 10, (7, 0), ([1.0, 2.0, 3.0], 3.0, 2.5)));
        let own = t.groups.get(1).unwrap().members[0].specific_move;
        assert_eq!(
            own,
            SpecificMove {
                active: 1,
                path_id: 7,
                reason: 0,
                goal: [1.0, 2.0, 3.0],
                timer: 3.0,
                consecutive_move_failures: 4,
                distance: 2.5,
                last_position: [5.0, 0.0, 5.0],
            }
        );
        assert!(t.set_specific_move(1, 10, (7, -1), ([1.0; 3], 1.0, 1.0)));
        let own = t.groups.get(1).unwrap().members[0].specific_move;
        assert_eq!(
            own,
            SpecificMove {
                path_id: -1,
                reason: -1,
                ..SpecificMove::default()
            }
        );
        assert!(!t.set_specific_move(1, 12, (0, 0), ([0.0; 3], 0.0, 0.0)));
        t.groups.get_mut(1).unwrap().members.push(Member {
            id: 99,
            ..Member::default()
        });
        assert!(!t.set_specific_move(1, 99, (0, 0), ([0.0; 3], 0.0, 0.0)));
    }

    /// A member on its slot (1e-7 or nearer, not a NaN) in state 3 clears its failures and
    /// takes the slot as its unit AI's goal; one more than 10 failures in is moved aside: its
    /// own move (reason 2, its place, 0.1 s), a stop and the group's flags bit 9.
    #[test]
    fn a_member_in_place_or_failing_too_often() {
        let mut t = world();
        let g = t.groups.get_mut(1).unwrap();
        g.current_formation.as_mut().unwrap().slots[0].transform_position = [5.0, 0.0, 5.0];
        g.current_formation.as_mut().unwrap().slots[0].state = 3;
        g.members[0].specific_move.consecutive_move_failures = 11;
        t.steer_to_slot(1, 0, 0, &[]).unwrap();
        let g = t.groups.get(1).unwrap();
        assert_eq!(g.members[0].specific_move.consecutive_move_failures, 0);
        assert_eq!(unit(&t, 10).ai.as_ref().unwrap().state_position, [5.0, 5.0]);
        let mut t = world();
        t.groups.get_mut(1).unwrap().members[0]
            .specific_move
            .consecutive_move_failures = 11;
        t.steer_to_slot(1, 0, 0, &[]).unwrap();
        let g = t.groups.get(1).unwrap();
        let own = g.members[0].specific_move;
        assert_eq!((own.active, own.path_id, own.reason), (1, -1, ASIDE));
        assert_eq!(
            (own.goal, own.timer, own.distance),
            ([5.0, 0.0, 5.0], 0.1, 0.0)
        );
        assert_eq!(own.consecutive_move_failures, 11);
        assert_eq!(g.flags & 0x200, 0x200);
        assert!(mover(&t, 10).speed_stopped);
        assert_eq!(unit(&t, 10).ai.as_ref().unwrap().state_position, [0.0; 2]);
        // a NaN gap is not in place
        let mut t = world();
        t.players[1].objects[0].mover.as_mut().unwrap().moved_at[0] = f32::NAN;
        let g = t.groups.get_mut(1).unwrap();
        g.current_formation.as_mut().unwrap().slots[0].state = 3;
        g.members[0].specific_move.consecutive_move_failures = 11;
        t.steer_to_slot(1, 0, 0, &[]).unwrap();
        assert_eq!(
            t.groups.get(1).unwrap().members[0].specific_move.reason,
            ASIDE
        );
    }

    /// A member that is not free (its unit AI acts) stops while it moves without a `MoveTo`,
    /// clears its failures, and its slot in state 1 or 2 takes state 3; a slot in another
    /// state keeps it.
    #[test]
    fn a_busy_member_stops_and_its_slot_counts_it_there() {
        for (before, after) in [(1, 3), (2, 3), (0, 0), (4, 4)] {
            let mut t = world();
            t.players[1].objects[0].ai.as_mut().unwrap().current_action = 600;
            let g = t.groups.get_mut(1).unwrap();
            g.current_formation.as_mut().unwrap().slots[0].state = before;
            g.members[0].specific_move.consecutive_move_failures = 3;
            t.steer_to_slot(1, 0, 0, &[]).unwrap();
            assert_eq!(state(&t, 0), after);
            assert_eq!(
                t.groups.get(1).unwrap().members[0]
                    .specific_move
                    .consecutive_move_failures,
                0
            );
            assert!(mover(&t, 10).speed_stopped);
        }
    }

    /// A free member five tiles from its slot steers to it: the yaw turns it round, its speed
    /// is its top speed and it shows its walking sprite (whose speed it takes), the group's
    /// flags take bit 9 and its unit AI's goal is the slot.
    #[test]
    fn a_far_member_turns_and_walks_to_its_slot() {
        let mut t = world();
        t.steer_to_slot(1, 0, 0, &[]).unwrap();
        let m = mover(&t, 10);
        assert!(0.99 < m.forward[0]);
        assert_eq!(m.velocity.unwrap().velocity[2], 0.8);
        assert_eq!(m.stats.speed, 0.9);
        assert_eq!(t.groups.get(1).unwrap().flags & 0x200, 0x200);
        assert_eq!(state(&t, 0), 0);
        assert_eq!(
            unit(&t, 10).ai.as_ref().unwrap().state_position,
            [10.0, 5.0]
        );
    }

    /// The speed by the gap to the slot: 4 tiles, 1.2 times the top speed, running; 1.5 tiles,
    /// the top speed plus 0.2 of it times 1.5 / 3; within five radii (0.5), the top speed,
    /// walking; within the radius (0.1), the group's top speed, walking.
    #[test]
    fn a_near_member_catches_up_or_takes_the_group_s_speed() {
        let speed = 0.8f32;
        let cases = [
            (4.0, speed * CATCH_UP, 1.5, 0x200),
            (
                1.5,
                speed + ((speed * CATCH_UP - speed) * 1.5) / CATCH_UP_SPAN,
                1.5,
                0x200,
            ),
            (0.5, speed, 0.9, 0),
            (0.1, 0.9, 0.9, 0),
        ];
        for (gap, v, shown, flags) in cases {
            let mut t = world();
            let f = t
                .groups
                .get_mut(1)
                .unwrap()
                .current_formation
                .as_mut()
                .unwrap();
            f.slots[0].transform_position = [5.0 + gap, 0.0, 5.0];
            t.steer_to_slot(1, 0, 0, &[]).unwrap();
            let m = mover(&t, 10);
            assert_eq!(m.velocity.unwrap().velocity[2], v, "{gap}");
            assert_eq!(m.stats.speed, shown, "{gap}");
            assert_eq!(t.groups.get(1).unwrap().flags & 0x200, flags, "{gap}");
        }
    }

    /// A member whose step would pass its slot (state 2) stops and, its group neither
    /// patrolling nor walking a path, shows its standing sprite.
    #[test]
    fn a_member_about_to_pass_its_slot_stops() {
        let mut t = world();
        let f = t
            .groups
            .get_mut(1)
            .unwrap()
            .current_formation
            .as_mut()
            .unwrap();
        f.slots[0].transform_position = [5.05, 0.0, 5.0];
        t.steer_to_slot(1, 0, 0, &[]).unwrap();
        assert_eq!(state(&t, 0), 2);
        let m = mover(&t, 10);
        assert!(m.speed_stopped);
        assert_eq!(m.velocity.unwrap().velocity[2], 0.0);
        assert_eq!(m.stats.speed, 0.0);
    }

    /// A slot whose time is past: state 1 becomes 2 and the member stops; in state 2 a moving
    /// group's member only shows standing and the slot takes state 3; a still group's member
    /// with a way there (the long pather, the unit without a height) stands on it facing the
    /// group's forward. A slot in state 3 shows standing.
    #[test]
    fn a_due_slot_takes_its_member() {
        let mut t = world();
        let g = t.groups.get_mut(1).unwrap();
        g.motion.velocity = [0.0, 0.0, 1.0];
        let s = &mut g.current_formation.as_mut().unwrap().slots[0];
        s.time = 50;
        s.state = 1;
        t.steer_to_slot(1, 0, 0, &[]).unwrap();
        assert_eq!(state(&t, 0), 3);
        assert!(mover(&t, 10).speed_stopped);
        assert_eq!(unit(&t, 10).position, [5.0, 5.0, 0.0]);
        let mut t = world();
        t.players[1].objects[0].mover.as_mut().unwrap().stats.radius[2] = 0.0;
        t.spatial.insert(
            10,
            Placed {
                obstruction: None,
                player: 1,
                ..post(5.0, 5.0)
            },
        );
        let s = &mut t
            .groups
            .get_mut(1)
            .unwrap()
            .current_formation
            .as_mut()
            .unwrap()
            .slots[0];
        s.time = 50;
        s.state = 2;
        t.steer_to_slot(1, 0, 0, &[]).unwrap();
        assert_eq!(state(&t, 0), 3);
        assert_eq!(unit(&t, 10).position, [10.0, 5.0, 0.0]);
        let m = mover(&t, 10);
        assert_eq!(m.moved_at, [10.0, 5.0, 0.0]);
        assert_eq!(m.last_move, 100);
        assert_eq!(m.forward, [1.0, 0.0, 0.0]);
        assert_eq!(m.right, [0.0, 0.0, -1.0]);
        assert_eq!(t.spatial.get(10).unwrap().position, [10.0, 5.0, 0.0]);
        // in state 3 the member only shows standing, its goal left
        let mut t = world();
        let s = &mut t
            .groups
            .get_mut(1)
            .unwrap()
            .current_formation
            .as_mut()
            .unwrap()
            .slots[0];
        s.time = 50;
        s.state = 3;
        t.steer_to_slot(1, 0, 0, &[]).unwrap();
        assert_eq!(mover(&t, 10).stats.speed, 0.0);
        assert_eq!(unit(&t, 10).ai.as_ref().unwrap().state_position, [0.0; 2]);
    }

    /// A record on the line to the slot is the way round, which the slice does not model.
    #[test]
    fn a_blocked_way_to_the_slot_stops() {
        let mut t = world();
        t.spatial.insert(50, post(7.5, 5.0));
        assert_eq!(t.steer_to_slot(1, 0, 0, &[]), Err(BLOCKED_WAY));
        // the update's list ignored: clear
        let mut t = world();
        t.spatial.insert(50, post(7.5, 5.0));
        assert_eq!(t.steer_to_slot(1, 0, 0, &[50]), Ok(()));
    }

    /// The members' moves: a member whose own move is active with a clear line to its slot
    /// stands and its move ends before it steers; one with a record in the way counts the
    /// move's timer down and waits while the timer and the distance are above 0, then ends
    /// it too; a member in no slot that moves stops.
    #[test]
    fn the_members_moves_end_wait_or_stop() {
        let mut t = world();
        t.spatial.insert(50, post(6.0, 7.0));
        let mut stray = member(12, 20.0, 20.0);
        stray.mover.as_mut().unwrap().velocity = Some(crate::movement::Velocity {
            velocity: [0.0, 0.0, 0.5],
            acceleration: [0.0; 3],
        });
        t.players[1].objects.push(stray);
        let g = t.groups.get_mut(1).unwrap();
        g.members.push(Member {
            id: 12,
            ..Member::default()
        });
        for m in &mut g.members[..2] {
            m.specific_move = SpecificMove {
                active: 1,
                path_id: 3,
                reason: 0,
                timer: 1.0,
                distance: 2.0,
                ..SpecificMove::default()
            };
        }
        t.members_to_slots(1, &[]).unwrap();
        let g = t.groups.get(1).unwrap();
        assert_eq!(
            g.members[0].specific_move,
            SpecificMove {
                path_id: -1,
                reason: -1,
                ..SpecificMove::default()
            }
        );
        assert_eq!(
            unit(&t, 10).ai.as_ref().unwrap().state_position,
            [10.0, 5.0]
        );
        assert_eq!(g.members[1].specific_move.timer, 1.0 - DT);
        assert_eq!(g.members[1].specific_move.reason, 0);
        assert_eq!(unit(&t, 11).ai.as_ref().unwrap().state_position, [0.0; 2]);
        assert!(mover(&t, 12).speed_stopped);
        assert!(!mover(&t, 11).speed_stopped);
        // the timer run out (or the distance at 0) ends the move, and the member steers on
        let mut t2 = world();
        t2.spatial.insert(50, post(6.0, 7.0));
        let g = t2.groups.get_mut(1).unwrap();
        g.members[1].specific_move = SpecificMove {
            active: 1,
            timer: 1.0,
            distance: 0.0,
            ..SpecificMove::default()
        };
        assert_eq!(t2.members_to_slots(1, &[50]), Ok(()));
        let own = t2.groups.get(1).unwrap().members[1].specific_move;
        assert_eq!((own.active, own.path_id, own.reason), (0, -1, -1));
        assert!(mover(&t2, 11).speed_stopped);
        assert_eq!(
            unit(&t2, 11).ai.as_ref().unwrap().state_position,
            [6.0, 9.0]
        );
    }
}
