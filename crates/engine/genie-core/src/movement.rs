//! A unit's movement (`RGE_Moving_Object`, MovingObject.cpp in 158041): the velocity step that
//! moves it along its forward vector, and the motion step that steers it along its path.
//!
//! Per tick, `RGE_Moving_Object::update` (`0x14114dbb0`) runs first: it brings the forward speed
//! toward the acceleration's target and moves the unit `dt × speed` along the forward vector it had
//! after the last tick ([`Mover::step_velocity`]). The actions run later in the same update; a
//! MoveTo calls the motion step (object slot 193, `0x141150f00`, [`Mover::motion`]), which turns the
//! unit toward its waypoint and sets the acceleration for the next tick.
//!
//! Vectors are the game's `BVector`s ([`fmath::Vec3`]: x, height, y).

use crate::fmath::{self, Vec3};
use crate::spatial::Spatial;
use crate::unit_ai::Unsupported;

/// The velocity data (`+0x868`): velocity and acceleration in the unit's own frame; component 2 is
/// along the forward vector.
#[derive(Clone, Debug, PartialEq)]
pub struct Velocity {
    pub velocity: Vec3,
    pub acceleration: Vec3,
}

/// One level of a unit's path data (`+0x858`; each level's parent at `+8`).
#[derive(Clone, Debug, PartialEq)]
pub struct PathData {
    /// `+0x14`, `mWaypointLevel`: 0 is the path the unit walks.
    pub level: u8,
    /// `+0x10`, `mLinkedPathType`: what a finished level hands its parent.
    pub linked_type: i32,
    /// The path's waypoints (`+0x18`, the path's `+0x438`..`+0x440`).
    pub points: Vec<Vec3>,
    /// The path's own flags (`+0xc`).
    pub path_flags: u32,
    /// `+0x24`: the index of the waypoint the unit walks to.
    pub waypoint: i32,
    /// `+0x28`: 2 wants a path, 3 walks, 4 and 5 arrived.
    pub state: i32,
    /// `+0x2c`: how near the target counts as there.
    pub range: f32,
    /// `+0x30`: the target object's id, −1 for a point.
    pub target: i32,
    /// `+0x34`, in seconds.
    pub pause_time: f32,
    /// `+0x3c`: bit 0 a waypoint was reached, bit 1 it was the waypoint itself.
    pub flags: u32,
}

/// What movement reads of the unit's master.
#[derive(Clone, Debug, PartialEq)]
pub struct MoveStats {
    /// The top speed (object slot 65, `0x141202c70`): the master's speed (`+0x740`), times the
    /// current sprite's speed factor when that is above 0.
    pub speed: f32,
    /// Master `+0x774`: the most the unit turns per second.
    pub turn_rate: f32,
    /// Master `+0x76c` and `+0x770`: the turn radius is `turn_radius × speed / turn_radius_speed`.
    pub turn_radius: f32,
    pub turn_radius_speed: f32,
    /// Master `+0x5d8`, `+0x5dc`, `+0x5e0`.
    pub radius: [f32; 3],
}

/// A moving unit's movement state.
#[derive(Clone, Debug, PartialEq)]
pub struct Mover {
    /// `+0x830`, a unit vector on the ground.
    pub forward: Vec3,
    /// `+0x83c`.
    pub right: Vec3,
    pub velocity: Option<Velocity>,
    /// The path data, the level the unit walks first, each next one its parent.
    pub paths: Vec<PathData>,
    /// `+0x878`: moveForward's distance factor.
    pub factor: f32,
    pub stats: MoveStats,
}

/// The moving unit, as the motion step needs it besides its [`Mover`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Me {
    pub id: i32,
    /// x, y, z on the map.
    pub position: [f32; 3],
}

/// What the motion step returns to the MoveTo action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    /// 0: still on the way.
    Moving,
    /// 1: the unit cannot move or has no path.
    Done,
    /// 2: it arrived.
    Arrived,
}

/// The gap below which the game takes a distance as zero (`DAT_142eb0c60`).
const EPSILON: f32 = 1e-7;
/// The world's up axis (`DAT_143cc2f30`), the yaw's axis.
const UP: Vec3 = [0.0, 1.0, 0.0];
/// How far inside the map's edge a moving unit's footprint stays (`DAT_142eb99cc`), and how far
/// below the edge a position stays (`DAT_142eb1e9c`).
const EDGE_GAP: f32 = 0.01;
const EDGE_POSITION_GAP: f32 = 0.001;
/// The class the motion step measures by its annexes (`-0xe85` encoded).
const GATE: i16 = 39;

/// A position on the map as the game's vector (x, height, y), and back.
fn vector([x, y, z]: [f32; 3]) -> Vec3 {
    [x, z, y]
}

fn map_position([x, z, y]: Vec3) -> [f32; 3] {
    [x, y, z]
}

impl PathData {
    /// The waypoint at `index` (`FUN_140f9bcd0`); the game returns a static zero point outside the
    /// path, which the slice does not follow.
    fn point(&self, index: i32) -> Result<Vec3, Unsupported> {
        usize::try_from(index)
            .ok()
            .and_then(|i| self.points.get(i))
            .copied()
            .ok_or(Unsupported("a waypoint index outside its path"))
    }
}

/// The distance between two footprints on the ground (`FUN_14118f680`): per axis the centre
/// distance minus both radii, at least 0, then the length of the two.
pub fn footprint_gap(a: [f32; 3], a_radius: [f32; 2], b: [f32; 3], b_radius: [f32; 2]) -> f32 {
    let x = ((b[0] - a[0]).abs() - (b_radius[0] + a_radius[0])).max(0.0);
    let y = ((b[1] - a[1]).abs() - (b_radius[1] + a_radius[1])).max(0.0);
    fmath::sqrt(x * x + y * y)
}

impl Mover {
    /// The velocity part of `RGE_Moving_Object::update` (`FUN_140f9ba80`): the forward speed moves
    /// toward the acceleration's target, capped by the top speed (object slot 188, `0x14114f300`),
    /// then the unit moves `dt × speed` along its forward vector (slot 182, `0x141158490`). Returns
    /// the position after it.
    pub fn step_velocity(&mut self, position: [f32; 3], dt: f32, map: [i32; 2]) -> [f32; 3] {
        let Some(v) = self.velocity.as_mut() else {
            return position;
        };
        let wanted = v.acceleration[2] * dt + v.velocity[2];
        let speed = if self.stats.speed < wanted {
            self.stats.speed
        } else if wanted < 0.0 {
            0.0
        } else {
            wanted
        };
        v.velocity[2] = speed;
        self.move_forward(dt * speed, position, map)
    }

    /// `moveForward` (slot 182): `distance × factor` along the forward vector, through the
    /// position setter; a distance below 1e-7 moves nothing.
    fn move_forward(&self, distance: f32, position: [f32; 3], map: [i32; 2]) -> [f32; 3] {
        let d = distance * self.factor;
        if d.abs() < EPSILON {
            return position;
        }
        let to = fmath::add(vector(position), fmath::scale(d, self.forward));
        self.set_position(map_position(to), map)
    }

    /// The position setter (slot 11 -> `FUN_141161950` -> `0x1411a58b0`): a unit that can move
    /// keeps its footprint 0.01 inside the map, then any position stays 0.001 below the map's size.
    fn set_position(&self, [x, y, z]: [f32; 3], map: [i32; 2]) -> [f32; 3] {
        let inside = |v: f32, radius: f32, size: i32| {
            let low = radius + EDGE_GAP;
            if v < low {
                low
            } else if size as f32 - low < v {
                (size as f32 - radius) - EDGE_GAP
            } else {
                v
            }
        };
        let (x, y) = if EPSILON < self.stats.speed {
            (
                inside(x, self.stats.radius[0], map[0]),
                inside(y, self.stats.radius[1], map[1]),
            )
        } else {
            (x, y)
        };
        let on_map = |v: f32, size: i32| {
            let v = v.max(0.0);
            if size as f32 <= v {
                size as f32 - EDGE_POSITION_GAP
            } else {
                v
            }
        };
        [on_map(x, map[0]), on_map(y, map[1]), z]
    }

    /// The motion step (object slot 193 -> `0x141150f00`, `doIntelligentMovementUpdate`) along the
    /// path a walking unit follows: toward the waypoint in a straight line, turning to face it.
    /// Only the walk of a level in state 3 is modelled; the waypoint arrivals, the path requests of
    /// state 2, the arrivals of states 4 and 5, unit groups and obstruction avoidance are not.
    pub fn motion(&mut self, me: &Me, dt: f32, world: &Spatial) -> Result<Motion, Unsupported> {
        if self.stats.speed == 0.0 {
            return Ok(Motion::Done);
        }
        let Some(level) = self.paths.first_mut() else {
            return Ok(Motion::Done);
        };
        if 0.0 < level.pause_time {
            level.pause_time -= dt;
            if 0.0 < level.pause_time {
                return Err(Unsupported("a paused path (stopMoving)"));
            }
            level.pause_time = 0.0;
        }
        let level = &self.paths[0];
        let target = match level.target {
            -1 => None,
            id => Some(
                world
                    .get(id)
                    .ok_or(Unsupported("a path target that is gone"))?,
            ),
        };
        if target.is_some_and(|t| t.class == GATE) {
            return Err(Unsupported("a path to a gate (its annexes)"));
        }
        if level.flags & 1 != 0 {
            return Err(Unsupported("a walker reached its waypoint"));
        }
        match level.state {
            4 | 5 => return Err(Unsupported("a path level that arrived (states 4 and 5)")),
            2 => {
                return Err(Unsupported(
                    "a path level that wants a path (the path request)",
                ));
            }
            _ => {}
        }
        let Some(target) = target else {
            return Err(Unsupported("a walk to a point"));
        };
        let position = vector(me.position);
        let waypoint = level.point(level.waypoint)?;
        let mut to = fmath::sub(position, waypoint);
        to[1] = 0.0;
        if fmath::sqrt(to[0] * to[0] + 0.0 + to[2] * to[2]) < EPSILON {
            return Err(Unsupported("a walker on its waypoint"));
        }
        let direction = self.direction_to(position, waypoint);
        let projected = self.projected(position, direction, dt);
        let reach = reach(waypoint, position, projected);
        if reach == Reach::Away {
            return Err(Unsupported("a walker moving away from its waypoint"));
        }
        if reach == Reach::Reached {
            return Err(Unsupported("a step that reaches its waypoint"));
        }
        if self.stats.radius.iter().all(|&r| 0.0 < r) {
            let rect = self.query_rect(position, projected, world.map);
            let near = world.obstruction_candidates(rect, self.stats.radius[0], &[me.id]);
            if !near.is_empty() {
                return Err(Unsupported("a walker that may meet an obstruction"));
            }
        }
        let ground = [self.stats.radius[0], self.stats.radius[1]];
        let gap = footprint_gap(me.position, ground, target.position, target.radius) - level.range;
        if gap < EPSILON {
            return Err(Unsupported("a walker that reached its target"));
        }
        self.steer(direction, dt);
        self.control_speed(position, waypoint);
        Ok(Motion::Moving)
    }

    /// The unit vector on the ground from `from` to `to` (slot 194, `0x14114f210`); the forward
    /// vector when the two are less than 1e-7 apart or on the same point.
    fn direction_to(&self, from: Vec3, to: Vec3) -> Vec3 {
        if from[0] != to[0] || from[2] != to[2] {
            let x = to[0] - from[0];
            let z = to[2] - from[2];
            let length = fmath::sqrt(x * x + 0.0 + z * z);
            if EPSILON <= length {
                return [x / length, 0.0, z / length];
            }
        }
        self.forward
    }

    /// Where this tick's speed would take the unit along `direction` (slot 199, `0x141161860`):
    /// the forward speed after one more step of acceleration, capped by the top speed.
    fn projected(&self, position: Vec3, direction: Vec3, dt: f32) -> Vec3 {
        let Some(v) = &self.velocity else {
            return position;
        };
        let mut next = fmath::add(v.velocity, fmath::scale(dt, v.acceleration));
        if self.stats.speed < next[2] || next[2] <= 0.0 {
            next[2] = self.stats.speed;
        }
        fmath::add(position, fmath::scale(dt * next[2], direction))
    }

    /// The obstruction query's rectangle (`FUN_14114cc60`): the step's two ends, each held within
    /// 0.1 of the map's far edges, then ordered (`FUN_141051900`).
    fn query_rect(&self, from: Vec3, to: Vec3, map: [i32; 2]) -> [f32; 4] {
        let far = |size: i32| size as f32 - 0.1;
        let hold = |v: f32, max: f32| if v < 0.0 { 0.0 } else { v.min(max) };
        let (x0, y0) = (hold(from[0], far(map[0])), hold(from[2], far(map[1])));
        let (x1, y1) = (hold(to[0], far(map[0])), hold(to[2], far(map[1])));
        [x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1)]
    }

    /// Turn toward `direction` (slot 195, `0x14114f630`, then the yaw, slot 184, `0x141163140`):
    /// by the angle between the flat forward vector and the direction, at most the turn rate times
    /// dt, about the up axis; then the forward vector and its right are renormalised (slot 173,
    /// `0x14115d150`, which rebuilds the right vector from up and forward).
    fn steer(&mut self, direction: Vec3, dt: f32) {
        let mut flat = self.forward;
        flat[1] = 0.0;
        let flat = fmath::normalize(flat);
        let angle = fmath::angle_between(direction, flat);
        let most = self.stats.turn_rate * dt;
        let mut turn = if angle <= most { angle } else { most };
        if 0.0 < fmath::cross(direction, flat)[1] {
            turn = -turn;
        }
        if self.paths[0].flags & 1 != 0 && turn == 0.0 {
            return;
        }
        let m = fmath::rotation(turn, UP);
        let forward = fmath::normalize(fmath::rotate(&m, self.forward));
        self.forward = forward;
        self.right = fmath::normalize(fmath::cross(UP, forward));
    }

    /// The speed control toward the waypoint (slot 198, `0x141150a80`): accelerate to the top speed
    /// unless the waypoint lies inside the turning circle, where the unit brakes.
    fn control_speed(&mut self, position: Vec3, waypoint: Vec3) {
        let Some(v) = self.velocity.as_mut() else {
            self.velocity = Some(Velocity {
                velocity: [0.0; 3],
                acceleration: [0.0, 0.0, f32::MAX],
            });
            return;
        };
        let d = fmath::sub(waypoint, position);
        let length = fmath::sqrt(d[1] * d[1] + d[0] * d[0] + d[2] * d[2]);
        let toward = [d[0] / length, d[1] / length, d[2] / length];
        let angle = fmath::angle_between(self.forward, toward);
        let radius = (self.stats.turn_radius * v.velocity[2]) / self.stats.turn_radius_speed;
        if 0.0 < v.velocity[2] && EPSILON < angle.abs() {
            let mut side = self.right;
            if 0.0 < fmath::cross(toward, self.forward)[1] {
                side = side.map(|c| -c);
            }
            let centre = [
                side[0] * radius + position[0],
                side[1] * radius + position[1],
                side[2] * radius + position[2],
            ];
            if fmath::length(fmath::sub(waypoint, centre)) < radius {
                v.acceleration[2] = -f32::MAX;
                return;
            }
        }
        if self.stats.speed <= v.velocity[2] {
            return;
        }
        v.acceleration[2] = f32::MAX;
    }
}

/// How a step relates to its waypoint (slot 196, `0x14114f400`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reach {
    /// 0: the step ends short of it.
    Short,
    /// 1: the step reaches or passes it.
    Reached,
    /// 2: the unit moves away from it.
    Away,
}

/// Compare the distances on the ground: waypoint to position, waypoint to the step's end, and the
/// step's own length.
fn reach(waypoint: Vec3, position: Vec3, projected: Vec3) -> Reach {
    let mut now = fmath::sub(waypoint, position);
    let mut after = fmath::sub(waypoint, projected);
    let step = fmath::sub(projected, position);
    now[1] = 0.0;
    after[1] = 0.0;
    let (now, after, step) = (
        fmath::length(now),
        fmath::length(after),
        fmath::length(step),
    );
    if EPSILON <= after {
        if step <= now {
            return Reach::Short;
        }
        if now < after {
            return Reach::Away;
        }
    }
    Reach::Reached
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spatial::{Obstruction, Placed};

    const DT: f32 = 0.02;

    fn stats() -> MoveStats {
        MoveStats {
            speed: 0.8,
            turn_rate: f32::MAX,
            turn_radius: 0.0,
            turn_radius_speed: f32::MAX,
            radius: [0.2, 0.2, 0.5],
        }
    }

    fn level(level: u8, points: &[[f32; 2]], state: i32) -> PathData {
        PathData {
            level,
            linked_type: if level == 0 { 1 } else { 0 },
            points: points.iter().map(|&[x, y]| [x, 0.0, y]).collect(),
            path_flags: if level == 0 { 6 } else { 4 },
            waypoint: 1,
            state,
            range: 0.1,
            target: 10028,
            pause_time: 0.0,
            flags: 0,
        }
    }

    /// MarvinBlue/426048731's builder 9177 as the game saved it after tick 679 (update 680): its
    /// first path toward the house foundation 10028, facing along it, not yet moving.
    fn builder_9177() -> Mover {
        Mover {
            forward: [0.8944272, 0.0, -0.44721362],
            right: [-0.44721362, 0.0, -0.8944272],
            velocity: Some(Velocity {
                velocity: [0.0; 3],
                acceleration: [0.0, 0.0, f32::MAX],
            }),
            paths: vec![
                level(0, &[[28.0, 89.0], [30.0, 88.0]], 3),
                level(2, &[[28.0, 89.0], [30.0, 88.0]], 0),
            ],
            factor: 1.0,
            stats: stats(),
        }
    }

    fn world() -> Spatial {
        let mut s = Spatial::new([120, 120]);
        let unit = |x, y| Placed {
            position: [x, y, 0.0],
            radius: [0.2, 0.2],
            class: 4,
            player: 2,
            master: 0,
            obstruction_type: 0,
            buildable_on: 0,
            tall: true,
            obstruction: Some(Obstruction { kind: 5, class: 2 }),
        };
        s.insert(9175, unit(30.0, 89.0));
        s.insert(9177, unit(28.0, 89.0));
        s.insert(
            10028,
            Placed {
                position: [30.0, 88.0, 0.0],
                radius: [1.0, 1.0],
                class: 3,
                player: 2,
                master: 0,
                obstruction_type: 0,
                buildable_on: 0,
                tall: true,
                obstruction: None,
            },
        );
        s
    }

    /// One tick of the walker: the velocity step, then the motion step from where it got.
    fn tick(m: &mut Mover, at: [f32; 3], world: &mut Spatial) -> ([f32; 3], Motion) {
        let at = m.step_velocity(at, DT, world.map);
        world.set_position(9177, at);
        let motion = m
            .motion(
                &Me {
                    id: 9177,
                    position: at,
                },
                DT,
                world,
            )
            .unwrap();
        (at, motion)
    }

    #[test]
    fn the_first_step_reaches_top_speed_and_moves_along_the_saved_forward_vector() {
        let mut m = builder_9177();
        let mut w = world();
        let (at, motion) = tick(&mut m, [28.0, 89.0, 0.0], &mut w);
        // the tick oracle's positions after update 681 (snapshot 680)
        assert_eq!(at, [28.014311, 88.992844, 0.0]);
        assert_eq!(motion, Motion::Moving);
        assert_eq!(m.velocity.as_ref().unwrap().velocity, [0.0, 0.0, 0.8]);
        assert_eq!(
            m.velocity.as_ref().unwrap().acceleration,
            [0.0, 0.0, f32::MAX]
        );
        assert_eq!(m.forward, [0.8944272, 0.0, -0.44721362]);
    }

    #[test]
    fn the_second_tick_turns_toward_the_waypoint_from_where_the_first_step_ended() {
        let mut m = builder_9177();
        let mut w = world();
        let (at, _) = tick(&mut m, [28.0, 89.0, 0.0], &mut w);
        let (at, _) = tick(&mut m, at, &mut w);
        assert_eq!(at, [28.028622, 88.98569, 0.0]);
        // after update 682 the game's forward vector points from there to (30, 88)
        assert_eq!(m.forward, [0.89458156, 0.0, -0.44690478]);
        assert_eq!(m.right, [-0.44690478, 0.0, -0.89458156]);
        let (at, _) = tick(&mut m, at, &mut w);
        assert_eq!(at, [28.042934, 88.97854, 0.0]);
    }

    #[test]
    fn the_gap_is_the_distance_between_the_footprints() {
        // MarvinBlue/426048731: a house at (30, 88), builders at (30, 89), (28, 89), (29, 99)
        let (villager, house) = ([0.2, 0.2], [1.0, 1.0]);
        let gap = |x, y| footprint_gap([x, y, 0.0], villager, [30.0, 88.0, 0.0], house);
        assert_eq!(gap(30.0, 89.0), 0.0);
        assert_eq!(gap(28.0, 89.0), 2.0f32 - 1.2);
        assert_eq!(gap(29.0, 99.0), (11.0f32 - 1.2).hypot(0.0));
    }

    #[test]
    fn a_unit_with_no_speed_or_no_path_is_done() {
        let mut m = builder_9177();
        m.stats.speed = 0.0;
        let me = Me {
            id: 9177,
            position: [28.0, 89.0, 0.0],
        };
        assert_eq!(m.motion(&me, DT, &world()), Ok(Motion::Done));
        let mut m = builder_9177();
        m.paths.clear();
        assert_eq!(m.motion(&me, DT, &world()), Ok(Motion::Done));
    }

    #[test]
    fn a_walker_that_may_meet_an_obstruction_is_not_modelled() {
        let mut m = builder_9177();
        let mut w = world();
        let mut close = w.get(9175).unwrap().clone();
        close.position = [28.3, 88.9, 0.0];
        w.insert(9175, close);
        let me = Me {
            id: 9177,
            position: [28.0, 89.0, 0.0],
        };
        assert!(m.motion(&me, DT, &w).is_err());
    }
}
