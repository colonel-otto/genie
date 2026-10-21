//! A new group's place and facing (FD-036): `SetInitialPosition` (`FUN_141039990`) on the first
//! clear point of a spiral round the centroid (`FUN_141007630`), `SetInitialOrientation`
//! (`FUN_141039680`) and the motion's orientation (`FUN_141049e10`).

use super::Motion;
use super::formation::on_map;
use super::motion::moved_vector;
use crate::fmath::{self, Vec3, cross, normalize};
use crate::turn::Turns;
use crate::unit_ai::Unsupported;

/// The spiral's steps (`FUN_141007630`'s table from `DAT_142f49460` and `DAT_142f49480`), x and y:
/// up y, up x, down y, down x.
const SPIRAL: [[f32; 2]; 4] = [
    crate::image::group::SPIRAL_FIRST[0],
    crate::image::group::SPIRAL_FIRST[1],
    crate::image::group::SPIRAL_LAST[0],
    crate::image::group::SPIRAL_LAST[1],
];
/// The points the spiral tries (`FUN_141007630`'s count from 1 below `0x191`).
const SPIRAL_POINTS: i32 = 400;
/// `FUN_141049e10` refuses a forward whose dot product with the up is below this
/// (`DAT_142f4c1a0`) ...
const SQUARE_MIN: f32 = crate::image::group::SQUARE_MIN;
/// ... or above this (`DAT_14330eb78`).
const SQUARE_MAX: f32 = crate::image::group::SQUARE_MAX;
/// A group whose members' forwards cancel out and that holds none faces x (`DAT_14330eb8c`).
const DEFAULT_FORWARD: f32 = crate::image::group::DEFAULT_FORWARD;

/// `FUN_141007630`'s spiral: where it is (x, y), its direction in [`SPIRAL`], its leg's length
/// and its count of steps (from 1).
struct Spiral {
    at: [f32; 2],
    direction: usize,
    leg: i32,
    count: i32,
}

impl From<[f32; 2]> for Spiral {
    fn from(at: [f32; 2]) -> Self {
        Self {
            at,
            direction: 0,
            leg: 1,
            count: 1,
        }
    }
}

impl Spiral {
    /// One step in the direction; it turns when the count divides by the leg's length, and the
    /// leg grows on a turn to y (an even direction).
    fn step(&mut self) {
        let [dx, dy] = SPIRAL[self.direction];
        self.at = [self.at[0] + dx, self.at[1] + dy];
        if self.count % self.leg == 0 {
            self.direction = (self.direction + 1) % SPIRAL.len();
            if self.direction.is_multiple_of(2) {
                self.leg += 1;
            }
        }
        self.count += 1;
    }
}

impl Motion {
    /// `FUN_141049e10`: the motion faces `forward` with `up`, both normalized, and their cross
    /// product `up × forward` (normalized) as its right; the new position takes all three, and
    /// with `both` (`param_4` 1) the position too. Motion flags bit 0 is set. False, with nothing
    /// changed, when the dot product of the two lies outside [`SQUARE_MIN`] to [`SQUARE_MAX`].
    pub fn set_orientation(&mut self, forward: Vec3, up: Vec3, both: bool) -> bool {
        let d = up[1] * forward[1] + up[0] * forward[0] + up[2] * forward[2];
        if d < SQUARE_MIN || SQUARE_MAX < d {
            return false;
        }
        let right = normalize(cross(up, forward));
        let (forward, up) = (normalize(forward), normalize(up));
        self.new_position[1] = forward;
        self.new_position[2] = up;
        self.new_position[3] = right;
        if both {
            self.position[1] = forward;
            self.position[2] = up;
            self.position[3] = right;
        }
        self.flags |= 1;
        true
    }
}

impl Turns {
    /// `SetInitialPosition` (`FUN_141039990`) of group `group`, as its `DAT_1439e60e4` (1, which
    /// nothing writes) has it: the stored positions lose their points, the group stands, at once
    /// and as its new position, on the first clear point round its centroid
    /// ([`Turns::clear_spot`]), faces as its members do ([`Turns::set_initial_orientation`]), and
    /// its flags take bit 10.
    ///
    /// # Errors
    ///
    /// A group the world does not hold, or what [`Turns::clear_spot`] does not model.
    ///
    /// # Panics
    ///
    /// Never: the group is the one found first.
    pub fn set_initial_position(&mut self, group: i32) -> Result<(), Unsupported> {
        let g = self.groups.get_mut(group).ok_or(Unsupported(
            "SetInitialPosition of a group the world does not hold",
        ))?;
        g.stored_positions.clear_points();
        let centroid = g.centroid;
        let at = self.clear_spot(group, centroid)?;
        let g = self.groups.get_mut(group).expect("the group found above");
        g.motion.new_position[0] = at;
        g.motion.position[0] = at;
        self.set_initial_orientation(group)?;
        let g = self.groups.get_mut(group).expect("the group found above");
        g.flags |= 0x400;
        Ok(())
    }

    /// `FUN_141007630`: the first point of a spiral from `centroid` that no obstruction record
    /// holds. The obstruction manager's queries take the terrain mask of the first member's
    /// master's restriction (`+0x606`, `FUN_141054630`) and ignore the members (`FUN_1410562e0`,
    /// left set after); each point, kept on the map ([`on_map`]), is a point query that leaves out
    /// the objects that move (flags 0x60, [`crate::spatial::Spatial::point_hit`]). From the
    /// centroid the spiral steps by [`SPIRAL`]'s directions in turn, a turn after each step that
    /// its count (from 1) divides by the leg's length, which grows on the turns to y; after
    /// [`SPIRAL_POINTS`] points, or for a group without members, the centroid. The point keeps the
    /// centroid's height.
    ///
    /// # Errors
    ///
    /// A first member the world does not hold (the game reads a null object) or of no terrain
    /// restriction, a map without its terrain, a centroid whose tile the first member's terrain
    /// lacks (`FUN_141052b90` puts a temporary record there that the routine never removes,
    /// `FUN_1410561f0`), and a point query near a record whose place the slice does not know.
    pub fn clear_spot(&self, group: i32, centroid: Vec3) -> Result<Vec3, Unsupported> {
        let g = self.groups.get(group).ok_or(Unsupported(
            "a clear spot for a group the world does not hold",
        ))?;
        let Some(first) = g.members.first() else {
            return Ok(centroid);
        };
        let (p, u) = self.object(first.id).ok_or(Unsupported(
            "a group's clear spot without its first member (a null object)",
        ))?;
        let mask = self.master_mask(p, u.life.master_id)?;
        if self
            .spatial
            .makes_tile_record([centroid[0], centroid[2]], mask)
        {
            return Err(Unsupported(
                "a group's clear spot on a tile its first member's terrain lacks (the tile record \
                 FUN_141007630 leaves)",
            ));
        }
        let members: Vec<i32> = g.members.iter().map(|m| m.id).collect();
        let mut spiral = Spiral::from([centroid[0], centroid[2]]);
        for _ in 0..SPIRAL_POINTS {
            let at = on_map([spiral.at[0], centroid[1], spiral.at[1]], self.spatial.map);
            if !self
                .spatial
                .point_hit([at[0], at[2]], 0.0, &members, true)?
            {
                return Ok(at);
            }
            spiral.step();
        }
        Ok(centroid)
    }

    /// `SetInitialOrientation` (`FUN_141039680`) of group `group`: its members' forwards (slot
    /// 0x648, `+0x830`) summed in their order and divided by the count of those the world holds,
    /// then normalized; when that is zero the first held member's forward, else
    /// ([`DEFAULT_FORWARD`], 0, 0). The motion faces it with up (0, 1, 0), position and new position
    /// alike ([`Motion::set_orientation`]).
    ///
    /// # Errors
    ///
    /// A group the world does not hold, or a held member without movement state.
    ///
    /// # Panics
    ///
    /// Never: the group is the one found first.
    pub fn set_initial_orientation(&mut self, group: i32) -> Result<(), Unsupported> {
        let g = self.groups.get(group).ok_or(Unsupported(
            "SetInitialOrientation of a group the world does not hold",
        ))?;
        let mut forwards = Vec::with_capacity(g.members.len());
        for m in &g.members {
            let Some((_, u)) = self.object(m.id) else {
                continue;
            };
            let mover = u.mover.as_ref().ok_or(Unsupported(
                "a group member without movement state (its forward, slot 0x648)",
            ))?;
            forwards.push(mover.forward);
        }
        let mut sum = [0.0f32; 3];
        for f in &forwards {
            sum = [sum[0] + f[0], sum[1] + f[1], sum[2] + f[2]];
        }
        if !forwards.is_empty() {
            let n = forwards.len() as f32;
            sum = [sum[0] / n, sum[1] / n, sum[2] / n];
        }
        let squared = sum[0] * sum[0] + sum[1] * sum[1] + sum[2] * sum[2];
        let forward = if squared == 0.0 {
            forwards
                .first()
                .copied()
                .unwrap_or([DEFAULT_FORWARD, 0.0, 0.0])
        } else {
            let l = fmath::sqrt(squared);
            [sum[0] / l, sum[1] / l, sum[2] / l]
        };
        let g = self.groups.get_mut(group).expect("the group found above");
        let _ = g.motion.set_orientation(forward, [0.0, 1.0, 0.0], true);
        Ok(())
    }
}

impl Turns {
    /// `movePositionToFront` (`FUN_141025a20`) of group `group`, `ahead` (`param_2`, 100 in the
    /// update) the distance of the point ahead along the group's forward. The group's place is
    /// its own, or with motion flags bit 1 the members' centre (slot 0x520, summed over the
    /// members the world holds, over the members' count). A flanked group (locked type 8) whose
    /// lead (`+0xc4`, `FUN_1410465d0`) could stand there among the members (slot 0x270,
    /// [`crate::spatial::Spatial::point_clear_among`], with [`FRONT_SHARE`] of its radius)
    /// stands there. Else the members within the spacing ([`FRONT_SHARE`] of it but for a
    /// staggered lock, type 7) of the line along the forward through that place (on the
    /// ground, [`fmath::line_distance`]) are in front: of them the one nearest the point ahead
    /// on the ground (the only one, however far), else the member nearest that point. The
    /// group's stored positions lose their points ([`super::PointLine::clear_points`]) and it
    /// stands on the chosen member's place (slot 0x520, height included). Nothing changes when
    /// the world holds no member.
    ///
    /// # Errors
    ///
    /// The lead's master, restriction or the map's terrain missing ([`Turns::master_mask`]),
    /// and its point test near a record whose place the slice does not know.
    ///
    /// # Panics
    ///
    /// Never: the group is the one found first.
    pub fn move_position_to_front(&mut self, group: i32, ahead: f32) -> Result<(), Unsupported> {
        let Some(g) = self.groups.get(group) else {
            return Ok(());
        };
        let m = &g.motion;
        let forward = m.position[1];
        let front = fmath::add(fmath::scale(ahead, forward), m.position[0]);
        let held: Vec<(i32, Vec3)> = g
            .members
            .iter()
            .filter_map(|mb| self.object(mb.id).map(|(_, u)| (mb.id, moved_vector(u))))
            .collect();
        let centre = if m.flags & 2 == 0 {
            m.position[0]
        } else {
            let mut c = [0.0f32; 3];
            for &(_, p) in &held {
                c = [c[0] + p[0], c[1] + p[1], c[2] + p[2]];
            }
            let n = g.members.len() as f32;
            [c[0] / n, c[1] / n, c[2] / n]
        };
        if g.formation_locked_type == super::formation::FLANKED
            && let Some((p, u)) = self.object(m.lead)
        {
            let master = self.master_of(p, u.life.master_id)?;
            let mask = self.master_mask(p, u.life.master_id)?;
            let members: Vec<i32> = g.members.iter().map(|mb| mb.id).collect();
            if self.spatial.point_clear_among(
                m.lead,
                [centre[0], centre[2]],
                FRONT_SHARE * master.radius[0],
                mask,
                &members,
            )? {
                let g = self.groups.get_mut(group).expect("the group found above");
                g.motion.new_position[0] = centre;
                g.motion.position[0] = centre;
                return Ok(());
            }
        }
        let limit = if g.formation_locked_type == super::formation::STAGGERED {
            g.desired_spacing
        } else {
            g.desired_spacing * FRONT_SHARE
        };
        let through = [centre[0], 0.0, centre[2]];
        let mut in_front: Vec<(i32, Vec3)> = Vec::new();
        let mut nearest = -1;
        let mut nearest_d = FARTHEST;
        for &(id, p) in &held {
            let q = [p[0], 0.0, p[2]];
            if fmath::line_distance(q, through, forward) < limit {
                in_front.push((id, p));
            }
            let d = ground_squared(q, front);
            if d < nearest_d {
                nearest = id;
                nearest_d = d;
            }
        }
        // of those in front the one nearest the point ahead: a lone one from the start
        let mut chosen = match in_front.as_slice() {
            [(id, _)] => *id,
            _ => -1,
        };
        let mut beside = FARTHEST;
        let mut chosen_d = FARTHEST;
        for &(id, p) in &in_front {
            let d = ground_squared(p, front);
            if d < chosen_d {
                chosen = id;
                beside = (p[2] - centre[2]) * (p[2] - centre[2]) + (p[0] - centre[0]) * (p[0] - centre[0]);
                chosen_d = d;
            }
        }
        let spot = match self.object(chosen) {
            Some((p, u))
                if !in_front.is_empty()
                    || ground_squared(moved_vector(u), front) <= nearest_d
                    || {
                        let r = self.master_of(p, u.life.master_id)?.radius[0];
                        beside <= (r + r) * (r + r)
                    } =>
            {
                Some(moved_vector(u))
            }
            _ => self.object(nearest).map(|(_, u)| moved_vector(u)),
        };
        let Some(at) = spot else {
            return Ok(());
        };
        let g = self.groups.get_mut(group).expect("the group found above");
        g.stored_positions.clear_points();
        g.motion.new_position[0] = at;
        g.motion.position[0] = at;
        Ok(())
    }

    /// The terrain classes master `master` of player `p` lets its objects stand on: its
    /// restriction (`+0x606`) in the map's table (`FUN_141054630`).
    ///
    /// # Errors
    ///
    /// No such master, a master without a restriction (the game indexes before the table) and a
    /// map without its terrain.
    pub(crate) fn master_mask(&self, p: usize, master: i16) -> Result<u8, Unsupported> {
        let restriction = usize::try_from(self.master_of(p, master)?.terrain_restriction)
            .map_err(|_| Unsupported("a terrain mask of a master without a restriction"))?;
        Ok(self
            .spatial
            .terrain
            .as_ref()
            .ok_or(Unsupported("a terrain mask without the map's terrain"))?
            .restriction_mask(restriction))
    }
}

/// `movePositionToFront`'s share of the spacing (`DAT_142e7f384`; the whole spacing for a
/// staggered lock) and of the lead's radius.
const FRONT_SHARE: f32 = crate::image::group::FRONT_SHARE;

/// The first best distance of `movePositionToFront`'s searches (`DAT_14330ebf4`).
const FARTHEST: f32 = crate::image::group::FARTHEST;

/// `p − q` squared on the ground, the height difference zeroed (`FUN_14087e560`, then
/// `x² + 0 + y²`).
fn ground_squared(p: Vec3, q: Vec3) -> f32 {
    let d = fmath::sub(p, q);
    d[0] * d[0] + 0.0 + d[2] * d[2]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `FUN_141049e10` facing x with up y: right is `y × x`, (0, 0, -1), the constructor's.
    #[test]
    fn an_orientation_takes_the_cross_product_of_up_and_forward_as_its_right() {
        let mut m = Motion::new();
        m.flags = 0;
        assert!(m.set_orientation([2.0, 0.0, 0.0], [0.0, 1.0, 0.0], true));
        assert_eq!(m.position[1], [1.0, 0.0, 0.0]);
        assert_eq!(m.position[3], [0.0, 0.0, -1.0]);
        assert_eq!(m.new_position[3], [0.0, 0.0, -1.0]);
        assert_eq!(m.flags, 1);
        // a forward too far from square to the up is refused
        assert!(!m.set_orientation([1.0, 0.1, 0.0], [0.0, 1.0, 0.0], true));
    }

    /// A unit at `[x, y]` (the unit's order: x, y, height) of master 4.
    fn unit(id: i32, x: f32, y: f32) -> crate::turn::Unit {
        let mut u = crate::turn::Unit {
            id,
            position: [x, y, 0.0],
            ..crate::turn::Unit::default()
        };
        u.life.master_id = 4;
        u
    }

    /// A 20 by 20 map of terrain class 1, its objects on the obstruction manager, and group 1 of
    /// them at (10, 10) facing x, spacing 2, two stored points.
    fn front_world(units: Vec<crate::turn::Unit>) -> Turns {
        use crate::spatial::{Obstruction, Placed, Spatial, Tiles};
        use crate::terrain::{Terrain, TerrainKind};
        let mut t = Turns::new(
            0,
            1,
            0,
            vec![0],
            vec![crate::turn::Player {
                objects: units.clone(),
                ..crate::turn::Player::default()
            }],
        );
        t.masters = vec![crate::masters::Masters::new([crate::masters::Master {
            id: 4,
            radius: [0.4, 0.4],
            terrain_restriction: 0,
            ..crate::masters::Master::default()
        }])];
        t.spatial = Spatial::with_tiles([20, 20], Tiles::new(20, 20, vec![1; 400]));
        let kind = TerrainKind {
            shown: true,
            flags: 1,
            forest: false,
        };
        t.spatial.terrain = Some(Terrain::new(
            20,
            20,
            vec![(0, 255, -1); 400],
            vec![kind],
            vec![vec![1.0]],
        ));
        for u in &units {
            t.spatial.insert(
                u.id,
                Placed {
                    position: u.position,
                    radius: [0.4, 0.4],
                    class: 6,
                    player: 0,
                    master: 4,
                    obstruction_type: 0,
                    buildable_on: 0,
                    tall: true,
                    speed: 0.0,
                    unknown: None,
                    obstruction: Some(Obstruction { kind: 5, class: 2 }),
                },
            );
        }
        let mut g = crate::group::UnitGroup::new(1);
        g.members = units
            .iter()
            .map(|u| crate::group::Member {
                id: u.id,
                ..crate::group::Member::default()
            })
            .collect();
        g.motion.position[0] = [10.0, 0.0, 10.0];
        g.motion.position[1] = [1.0, 0.0, 0.0];
        g.motion.flags = 0;
        g.desired_spacing = 2.0;
        g.stored_positions.points = vec![crate::group::LinePoint::default(); 2];
        g.stored_positions.length = 3.0;
        t.groups = crate::group::UnitGroups {
            groups: vec![g],
            ..crate::group::UnitGroups::default()
        };
        t
    }

    fn placed(t: &Turns) -> (Vec3, Vec3, usize) {
        let g = &t.groups.groups[0];
        (
            g.motion.position[0],
            g.motion.new_position[0],
            g.stored_positions.points.len(),
        )
    }

    /// `FUN_141025a20`: of the members within half the spacing (1) of the line along the forward
    /// through the group, the one nearest the point 100 ahead, (110, 10): 10 at (15, 10.5),
    /// before 11 at (13, 9.2); 12, far off the line, is nearer that point but not in front. The
    /// group stands on 10's place, its stored points gone, their length kept.
    #[test]
    fn the_group_moves_onto_the_member_in_front_nearest_the_point_ahead() {
        let mut t = front_world(vec![
            unit(10, 15.0, 10.5),
            unit(11, 13.0, 9.2),
            unit(12, 19.0, 15.0),
        ]);
        t.move_position_to_front(1, 100.0).unwrap();
        assert_eq!(placed(&t), ([15.0, 0.0, 10.5], [15.0, 0.0, 10.5], 0));
        assert_eq!(t.groups.groups[0].stored_positions.length, 3.0);
    }

    /// With no member in front the group moves onto the member nearest the point ahead; with
    /// none the world holds nothing changes.
    #[test]
    fn without_a_member_in_front_the_group_moves_onto_the_nearest() {
        let mut t = front_world(vec![unit(11, 13.0, 12.0), unit(12, 19.0, 15.0)]);
        t.move_position_to_front(1, 100.0).unwrap();
        assert_eq!(placed(&t), ([19.0, 0.0, 15.0], [19.0, 0.0, 15.0], 0));
        let mut t = front_world(Vec::new());
        t.groups.groups[0].members = vec![crate::group::Member {
            id: 99,
            ..crate::group::Member::default()
        }];
        t.move_position_to_front(1, 100.0).unwrap();
        assert_eq!(placed(&t), ([10.0, 0.0, 10.0], [0.0; 3], 2));
        // a group the world does not hold is left alone
        t.move_position_to_front(2, 100.0).unwrap();
    }

    /// A staggered lock (7) takes the whole spacing: 11 at 1.5 from the line is in front of it
    /// (and of no other lock), 12 nearer the point ahead only in the other.
    #[test]
    fn a_staggered_lock_takes_the_whole_spacing() {
        let units = vec![unit(11, 13.0, 11.5), unit(12, 19.0, 13.0)];
        let mut t = front_world(units.clone());
        t.groups.groups[0].formation_locked_type = crate::group::formation::STAGGERED;
        t.move_position_to_front(1, 100.0).unwrap();
        assert_eq!(placed(&t).0, [13.0, 0.0, 11.5]);
        let mut t = front_world(units);
        t.move_position_to_front(1, 100.0).unwrap();
        assert_eq!(placed(&t).0, [19.0, 0.0, 13.0]);
    }

    /// Motion flags bit 1 take the line through the members' centre, (13, 0, 12): 11 at
    /// (13, 12.2) lies on it, 10 at (6, 10.1) on the group's own line.
    #[test]
    fn with_flags_bit_1_the_line_runs_through_the_members_centre() {
        let units = vec![unit(10, 6.0, 10.1), unit(11, 13.0, 12.2), unit(12, 20.0, 13.7)];
        let mut t = front_world(units.clone());
        t.groups.groups[0].motion.flags = 2;
        t.move_position_to_front(1, 100.0).unwrap();
        assert_eq!(placed(&t).0, [13.0, 0.0, 12.2]);
        let mut t = front_world(units);
        t.move_position_to_front(1, 100.0).unwrap();
        assert_eq!(placed(&t).0, [6.0, 0.0, 10.1]);
    }

    /// A flanked group (8) whose lead could stand on the group's place among the members stays
    /// there, its stored points kept: member 11's record holds the place (grown by half the
    /// lead's radius, 0.2) but the members are ignored. Object 50 of no member there sends it to
    /// the front: of 10 and 11, both in front, 10 is nearer the point ahead.
    #[test]
    fn a_flanked_group_stays_where_its_lead_could_stand() {
        let units = vec![unit(10, 15.0, 10.5), unit(11, 10.2, 10.1)];
        let flanked = |t: &mut Turns| {
            t.groups.groups[0].formation_locked_type = crate::group::formation::FLANKED;
            t.groups.groups[0].motion.lead = 10;
        };
        let mut t = front_world(units.clone());
        flanked(&mut t);
        t.move_position_to_front(1, 100.0).unwrap();
        assert_eq!(placed(&t), ([10.0, 0.0, 10.0], [10.0, 0.0, 10.0], 2));
        let mut other = units[1].clone();
        other.id = 50;
        let mut t = front_world(vec![units[0].clone(), units[1].clone(), other]);
        flanked(&mut t);
        t.groups.groups[0].members.truncate(2);
        t.move_position_to_front(1, 100.0).unwrap();
        assert_eq!(placed(&t), ([15.0, 0.0, 10.5], [15.0, 0.0, 10.5], 0));
    }

    /// The spiral's first points from (10, 10): the leg's count and turns as `FUN_141007630`
    /// takes them (a turn after steps 1, 2, 4, 6, 9, 12, ...).
    #[test]
    fn the_spiral_turns_where_its_step_count_divides_by_the_leg() {
        let mut spiral = Spiral::from([10.0, 10.0]);
        let mut points = vec![spiral.at];
        for _ in 0..12 {
            spiral.step();
            points.push(spiral.at);
        }
        assert_eq!(
            points,
            [
                [10.0, 10.0],
                [10.0, 11.0],
                [11.0, 11.0],
                [11.0, 10.0],
                [11.0, 9.0],
                [10.0, 9.0],
                [9.0, 9.0],
                [9.0, 10.0],
                [9.0, 11.0],
                [9.0, 12.0],
                [10.0, 12.0],
                [11.0, 12.0],
                [12.0, 12.0],
            ]
        );
    }
}
