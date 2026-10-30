//! A group's motion (FD-036): `BMotion`'s update (`FUN_14104a090`, motion.cpp): its path step
//! (`FUN_14104a270`) over the path levels a unit's motion step keeps alike, planned for the
//! group's lead from the group's place, the walk toward the walked level's waypoint with its turn
//! (`FUN_141046270`, `FUN_14104c150`), then the velocity along the vectors.

use crate::GenieError;
use crate::groups::Motion;
use crate::math::fmath::{self, Vec3};
use crate::movement::Walker;
use crate::world::World;
use crate::world::slots::CheckedIndex;

mod levels;
mod request;
mod step;
mod walk;

use self::levels::{ground, looping_next, point, reset_level, set_flag};
pub(in crate::groups) use self::levels::{hold, passes_waypoint};
use self::request::request;
use self::step::Pass;
pub(crate) use self::step::{StepWorld, path_step};
use self::walk::{SHORT_IGNORED, steer};
#[cfg(test)]
use crate::map::spatial::Spatial;
#[cfg(test)]
use crate::movement::{PathData, Planning};

/// A move or a turn below this counts as none (`DAT_142eb0c60`).
const EPSILON: f32 = crate::math::image::group::EPSILON;
/// How near a point the walk counts as there (`DAT_142eb5690`).
const NEAR: f32 = crate::math::image::group::NEAR;
/// A waypoint or a target nearer than this takes the straight way when nothing stands on it
/// (`DAT_14330eb8c`).
const CLOSE: f32 = crate::math::image::group::CLOSE;
/// A path shorter than this is no path (`DAT_142eb99cc`).
const SHORT_LENGTH: f32 = crate::math::image::group::SHORT_LENGTH;
/// The short pather serves a waypoint this near (`DAT_142eb0f4c`).
const SHORT_REACH: f32 = crate::math::image::group::SHORT_REACH;
/// The members' centre pulls the group when it is this far (squared, `DAT_142eb0f48`) ...
const PULL_DISTANCE: f32 = crate::math::image::group::PULL_DISTANCE;
/// ... by this share (`DAT_142eae8d0`), the group keeping this much of its place
/// (`DAT_142ecb878`).
const PULL_SHARE: f32 = crate::math::image::group::PULL_SHARE;
const PULL_KEEP: f32 = crate::math::image::group::PULL_KEEP;
/// The passes of one step (`0x14 < local_7e0` ends it).
const PASSES: u32 = 20;
/// A level with fewer plans left than this gives up rather than wait (`FUN_140f95bf0 < 3`).
const LAST_TRIES: i32 = 3;
/// A patrolling group at its last waypoint pauses this long (the immediate `0x3f000000`).
const PATROL_PAUSE: f32 = 0.5;
/// A group without a path left pauses this long (the immediate `0x3dcccccd`).
const STUCK_PAUSE: f32 = 0.1;

/// What `BMotion` reads of the group's lead (`+0xc4`, `FUN_141016870`'s member).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Lead {
    /// What the planners read of it.
    pub(crate) walker: Walker,
    /// Its own place (x, y, height), from which the long pather measures its target.
    pub(crate) position: [f32; 3],
    /// Its master's radii on the ground.
    pub(crate) radius: [f32; 2],
    /// It moves (slot 0x208 not 0, or master 42), as the long pather reads it.
    pub(crate) moves: bool,
    /// Its master has a footprint and a height (`FUN_141194060` with 1): the short pather serves it.
    pub(crate) solid: bool,
    /// Its ignore units (`+0x670`).
    pub(crate) ignore_units: Vec<i32>,
    /// Its owner's index and whether that is a computer player (`FUN_141659cf0`).
    pub(crate) player: usize,
    pub(crate) computer: bool,
    /// Its group (`+0x6d0`), when the world holds it.
    pub(crate) group: Option<LeadGroup>,
}

/// What `BMotion` reads of the lead's group.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct LeadGroup {
    /// `mGroupFlags`: bit 11 marks a patrol.
    pub(crate) flags: u32,
    /// The members (`+0xf0`).
    pub(crate) members: Vec<i32>,
    /// The differentiator groups' members, at most 128 (`FUN_14100bbe0` with `0x80`).
    pub(crate) differentiator_units: Vec<i32>,
    /// The same, at most 255, as the long pather lowers them.
    pub(crate) lowered: Vec<i32>,
}

impl LeadGroup {
    const fn patrols(&self) -> bool {
        self.flags & 0x800 != 0
    }
}

impl World {
    /// Slot 0x280 (`FUN_141082d60`): the id of the object the unit's top action targets
    /// (`FUN_140a78450`: an id above -1 that the world holds), else -1.
    pub(crate) fn top_target_object(&self, u: &crate::objects::unit::Unit) -> i32 {
        u.top_target()
            .filter(|&t| -1 < t && self.object(t).is_some())
            .unwrap_or(-1)
    }
}

/// What `BMotion` reads of the group itself besides its motion: its owner is a computer player
/// (`FUN_141659cf0` on the group's `+0xdc`), and per member its top action's target (slot 0x280,
/// `FUN_141082d60`: -1 without one) and where it last moved to (slot 0x520, x, height, y), each
/// None for a member the world does not hold.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct GroupView {
    pub(crate) computer: bool,
    pub(crate) members: Vec<Option<(i32, Vec3)>>,
}

impl Motion {
    /// `FUN_141046600`: the motion moves (its velocity's length, `sqrt(x² + y² + z²)`, is above
    /// 0).
    #[must_use]
    pub fn moving(&self) -> bool {
        let v = self.velocity;
        0.0 < fmath::sqrt(v[0] * v[0] + v[1] * v[1] + v[2] * v[2])
    }

    /// `FUN_141046640`: the walked level waits for a path (its state 1 or 2); false without one.
    #[must_use]
    pub fn waits_to_move(&self) -> bool {
        self.paths.first().is_some_and(|l| matches!(l.state, 1 | 2))
    }

    /// `FUN_1410481f0`: with a walked level below another, the levels below the first of level
    /// 2 go (`FUN_140f98e80`, each alone); that level walks again from waypoint 1 in state 2 with
    /// 10 tries; the top reached without one is reset ([`reset_level`]). A lone walked level
    /// stays as it is.
    pub fn drop_to_high_level(&mut self) -> Result<(), GenieError> {
        if self.paths.len() < 2 {
            return Ok(());
        }
        while self.paths.checked(0)?.level != 2 && 1 < self.paths.len() {
            let _ = self.paths.remove(0);
        }
        let level = self.paths.checked_mut(0)?;
        if level.level == 2 {
            level.waypoint = 1;
            level.state = 2;
            level.continue_counter = 10;
        } else {
            reset_level(&mut self.paths)?;
        }
        Ok(())
    }

    /// The walk's stop (`LAB_14104bd7e`): the forward speed and its acceleration go to 0.
    const fn stand(&mut self) {
        self.acceleration[2] = 0.0;
        self.velocity[2] = 0.0;
    }

    /// `FUN_14104a050`: the motion stops ([`Motion::stand`]) and its walked level resets
    /// (`FUN_140f98e80`, its path and its parents gone, its repath time kept).
    pub(in crate::groups) fn stop_walk(&mut self) -> Result<(), GenieError> {
        self.stand();
        reset_level(&mut self.paths)
    }

    /// `BMotion::calculateYawAngle` (`FUN_141046270`): the angle from `forward` (`param_2`) to
    /// `direction` (`param_3`, `FUN_14087e650`); with `clamp` (`param_5` 1) at most `+0xa0`
    /// (the yaw per second moving) times `dt` (`param_4`), as `MINSS` takes it at 0x1410463e2
    /// (the limit when it is below the angle, else the angle, also when either is NaN); negated
    /// when `direction × forward` (`FUN_14087e4d0`) points up (`COMISS`/`JBE` at 0x141046400:
    /// not for a NaN). The warning for a `dt` of `f32::MAX` with a yaw per second above 1 and
    /// the one of the result are log lines only.
    #[must_use]
    pub fn calculate_yaw_angle(&self, forward: Vec3, direction: Vec3, dt: f32, clamp: bool) -> f32 {
        let mut angle = fmath::angle_between(direction, forward);
        if clamp {
            angle = fmath::minss(self.maximum_yaw_per_second_moving * dt, angle);
        }
        if 0.0 < fmath::cross(direction, forward)[1] {
            -angle
        } else {
            angle
        }
    }

    /// `FUN_14104c150`: a turn of `yaw` (above 1e-7 in size) about the new position's up: its
    /// forward and right turn (`FUN_14087e350`, `FUN_14087e2c0`) and are normalized
    /// (`FUN_141027d00`); motion flags bit 0.
    pub fn turn(&mut self, yaw: f32) {
        // only an ordered 1e-7 below the yaw's size turns: a NaN yaw does not
        if fmath::at_most_or_unordered(yaw.abs(), EPSILON) {
            return;
        }
        let r = fmath::rotation(yaw, self.new_position[2]);
        self.new_position[1] = fmath::normalize(fmath::rotate(&r, self.new_position[1]));
        self.new_position[3] = fmath::normalize(fmath::rotate(&r, self.new_position[3]));
        self.flags |= 1;
    }

    /// `BMotion`'s update (`FUN_14104a090`) after its path step ([`path_step`]): the forward
    /// speed grows by `dt` times its acceleration; the new position moves by `dt` times each
    /// speed along its vector (forward, right, up) when that is above 1e-7 in size (motion flags
    /// bit 0); then the position is the new position, all four vectors.
    pub fn integrate(&mut self, dt: f32) -> Result<(), GenieError> {
        self.velocity[2] += dt * self.acceleration[2];
        for (speed, along) in [
            (self.velocity[2], 1),
            (self.velocity[0], 3),
            (self.velocity[1], 2),
        ] {
            let d = dt * speed;
            if EPSILON < d.abs() {
                let v = fmath::scale(d, *self.new_position.checked(along)?);
                let p = &mut self.new_position[0];
                *p = [p[0] + v[0], v[1] + p[1], v[2] + p[2]];
                self.flags |= 1;
            }
        }
        self.position = self.new_position;
        Ok(())
    }
}

impl World {
    /// `FUN_1410465d0`: the object the world's table holds for `motion`'s lead (`+0xc4`,
    /// [`World::world_object`]). The constructor's lead -1 finds the object of id -1, a
    /// recyclable master's (a missile or a ghost), while one is in a player's list.
    pub(crate) fn lead_object(&self, motion: &Motion) -> Option<crate::groups::WorldObject<'_>> {
        self.world_object(motion.lead)
    }

    /// What `BMotion` reads of the lead `id` (a unit with movement state), None without one.
    fn lead_view(&self, id: i32) -> Option<Lead> {
        let (p, u) = self.object(id)?;
        let mover = u.body.mover()?;
        let group = self.groups.get(u.group.id).map(|g| LeadGroup {
            flags: g.flags,
            members: g.members.iter().map(|m| m.id).collect(),
            differentiator_units: self.groups.differentiator_units(g, SHORT_IGNORED),
            lowered: self.groups.group_units(u.group.id).unwrap_or_default(),
        });
        Some(Lead {
            walker: mover.walker(id),
            position: u.position(),
            radius: [mover.stats.radius[0], mover.stats.radius[1]],
            moves: mover.stats.speed != 0.0 || u.life.master_id == crate::movement::LONG_PATH_MOVER,
            solid: mover.stats.radius.iter().all(|&r| 0.0 < r),
            ignore_units: mover.ignore_units.clone(),
            player: p,
            computer: self.players.get(p).is_some_and(|q| q.computer),
            group,
        })
    }
}

#[cfg(test)]
mod tests;
