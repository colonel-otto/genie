//! A new group's place and facing (FD-036): `SetInitialPosition` (`FUN_141039990`) on the first
//! clear point of a spiral round the centroid (`FUN_141007630`), `SetInitialOrientation`
//! (`FUN_141039680`) and the motion's orientation (`FUN_141049e10`).

use super::Motion;
use super::formation::on_map;
use crate::fmath::{Vec3, cross, normalize};
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
        g.stored_positions.points.clear();
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
        let restriction = usize::try_from(self.master_of(p, u.life.master_id)?.terrain_restriction)
            .map_err(|_| Unsupported("a group's clear spot for a master without a restriction"))?;
        let mask = self
            .spatial
            .terrain
            .as_ref()
            .ok_or(Unsupported(
                "a group's clear spot without the map's terrain",
            ))?
            .restriction_mask(restriction);
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
            let l = crate::fmath::sqrt(squared);
            [sum[0] / l, sum[1] / l, sum[2] / l]
        };
        let g = self.groups.get_mut(group).expect("the group found above");
        let _ = g.motion.set_orientation(forward, [0.0, 1.0, 0.0], true);
        Ok(())
    }
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
