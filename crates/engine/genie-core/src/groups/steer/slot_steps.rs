//! A due slot's try, the steering on a clear line, and the setters of a member's state,
//! speed, facing, place and own move.

use crate::GenieError;
use crate::groups::SpecificMove;
use crate::groups::motion::{moved_vector, passes_waypoint};
use crate::groups::steer::{
    BUILDING_KIND, CATCH_UP, CATCH_UP_SPAN, LINKED_RADIUS, MOVES, SLOT_REACH,
};
use crate::math::fmath::{Vec3, at_most_or_unordered, below_or_unordered};
use crate::turn::Turns;

impl Turns {
    /// `FUN_141025fb0` for a slot whose time is past (`0x1410263e4`..`0x1410267c5`): true when
    /// it ends the routine (slot state 2 or 3). See [`Turns::steer_to_slot`].
    pub(super) fn slot_due(
        &mut self,
        group: i32,
        index: usize,
        slot: usize,
    ) -> Result<bool, GenieError> {
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
    pub(super) fn try_slot(&mut self, group: i32, id: i32, to: Vec3) -> Result<bool, GenieError> {
        let (p, u) = self.object(id).expect("the member is held");
        let mover = u.mover.as_ref().expect(MOVES);
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
                object: self.streams.get(crate::math::rng::Stream::Object),
                construction_rate: 0.0,
                groups: &self.groups,
            };
            let mut path = crate::pathing::tile_path::Path::default();
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
    pub(super) fn steer_member(
        &mut self,
        group: i32,
        slot: usize,
        id: i32,
        gap: f32,
    ) -> Result<(), GenieError> {
        let g = self.groups.get(group).expect("held");
        let to = g.current_formation.as_ref().expect("a formation").slots[slot].transform_position;
        let dt = self.dt_seconds;
        let (_, u) = self.object_mut(id).expect("the member is held");
        let facets = u.sprite.map(|s| crate::facing::SpriteFacets {
            count: s.graphic.facets,
            flag_1: s.graphic.flags & 2 != 0,
        });
        let at = moved_vector(u);
        let mover = u.mover.as_mut().expect(MOVES);
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
            let patrol = crate::groups::formation::top_path(&g.motion.paths)
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
    pub(super) fn set_state_position(&mut self, id: i32, to: Vec3) {
        if let Some((_, u)) = self.object_mut(id)
            && let Some(ai) = u.ai.as_mut()
        {
            ai.state_position = [to[0], to[2]];
        }
    }

    /// Slot `slot` of group `group`'s formation takes state `state`.
    pub(super) fn set_slot_state(&mut self, group: i32, slot: usize, state: i32) {
        let g = self.groups.get_mut(group).expect("held");
        g.current_formation.as_mut().expect("a formation").slots[slot].state = state;
    }

    /// Slot 0x5d0 of member `id`: it stops ([`crate::movement::Mover::stop`]).
    pub(super) fn stop_member(&mut self, id: i32) -> Result<(), GenieError> {
        if let Some((_, u)) = self.object_mut(id)
            && let Some(mover) = u.mover.as_mut()
        {
            mover.stop()?;
        }
        Ok(())
    }

    /// Slot 0x598 of member `id` ([`crate::movement::Mover::set_speed`]).
    pub(super) fn set_member_speed(&mut self, id: i32, speed: f32) {
        if let Some((_, u)) = self.object_mut(id)
            && let Some(mover) = u.mover.as_mut()
        {
            mover.set_speed(speed);
        }
    }

    /// Slot 0x568 of member `id` with group `group`'s forward and up vectors (`+0x14`,
    /// `+0x20`, [`crate::movement::Mover::set_orientation`]).
    pub(super) fn face_group(&mut self, group: i32, id: i32) {
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
    /// [`crate::vision::visibility::Visibility::link_building`]). The moved records of the objects
    /// fastened to a combat object (`+0x8b0`..`+0x8f0`), which the slice does not hold, are a
    /// unit's none.
    ///
    /// # Errors
    ///
    /// An object of a master the slice does not have.
    pub(super) fn place_member(&mut self, id: i32, [x, h, y]: Vec3) -> Result<(), GenieError> {
        let Some(place) = self.place(id) else {
            return Ok(());
        };
        let (p, master_id) = {
            let (p, u) = self.object(id).expect("placed");
            (p, u.life.master_id)
        };
        let kind = self.master_of(p, master_id)?.kind;
        let u = match place {
            crate::orders::Place::Live(p, i) => &mut self.players[p].objects[i],
            crate::orders::Place::Sleeping(p, i) => &mut self.players[p].sleeping[i],
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
