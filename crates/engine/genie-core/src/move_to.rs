//! The MoveTo action (`RGE_Action_Move_To`, type 1, vtable `0x142ed6030`): a unit walks to a target
//! object or a point. Builders, birds and wandering animals run it, as a sub-action or on its own.
//!
//! 158041: start (slot 5) `0x140aa2100`, update (slot 6) `0x140aa2a70`, set_state (slot 16)
//! `0x140aa29e0`. A MoveTo in state 3 asks for a path (`FUN_14114e410`) and turns to state 4; in
//! state 4 each update runs the motion step (object slot 193, [`Mover::motion`]).

use crate::movement::{Me, Motion, Mover, Planning, footprint_gap};
use crate::spatial::Spatial;
use crate::unit_ai::Unsupported;

/// The MoveTo action's type, which word 9 adds when it is a unit's top action.
pub const MOVE_TO: i16 = 1;

/// The MoveTo's state (`+0x18`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveTo {
    /// 1: at the target when it started.
    Arrived,
    /// 3: it starts to move (`0x140aa2100`).
    Start,
    /// 4: its first update asked for a path; the unit moves from the next update on.
    Moving,
}

impl MoveTo {
    /// The state's number, which word 9 adds.
    pub fn number(self) -> u8 {
        match self {
            MoveTo::Arrived => 1,
            MoveTo::Start => 3,
            MoveTo::Moving => 4,
        }
    }
}

/// What a walking unit's update needs: its movement state, where it is, and the world; the
/// planners and the world's time for a path request (None where the caller has none).
pub struct Walk<'a> {
    pub mover: Option<&'a mut Mover>,
    pub me: Me,
    pub dt: f32,
    pub world: &'a Spatial,
    pub plan: Option<Planning<'a>>,
}

/// Where a MoveTo goes: a target object or a point (`x, y, z` on the map).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Goal {
    Target(i32),
    Point([f32; 3]),
}

/// A starting MoveTo's update (state 3): unless the unit is within `range` of its goal already, it
/// asks for a path (`FUN_14114e410`) and turns to state 4, whose set_state runs the update again at
/// once ([`walk_on`]). A unit already in range (set_state 1) is not modelled.
pub fn start_walk(walk: Walk<'_>, goal: Goal, range: f32) -> Result<(), Unsupported> {
    let (point, target, radius) = match goal {
        Goal::Target(id) => {
            let t = walk
                .world
                .get_known(id)?
                .ok_or(Unsupported("a MoveTo whose target is gone"))?;
            (t.position, id, t.radius)
        }
        Goal::Point(p) => (p, -1, [0.0, 0.0]),
    };
    let mover = walk
        .mover
        .ok_or(Unsupported("a walker without movement state"))?;
    let ground = [mover.stats.radius[0], mover.stats.radius[1]];
    let gap = footprint_gap(walk.me.position, ground, point, radius);
    if gap * gap <= range * range {
        return Err(Unsupported(
            "a MoveTo in range when it starts (set_state 1)",
        ));
    }
    let now = walk.plan.as_ref().map_or(0, |p| p.now);
    mover.request_path(walk.me.position, point, range, target, now);
    walk_on(Walk {
        mover: Some(mover),
        ..walk
    })
}

/// A moving MoveTo's update (state 4): the motion step, which only the walk keeps going. An
/// arrival, a unit that cannot go on and a MoveTo without its path are not modelled.
pub fn walk_on(walk: Walk<'_>) -> Result<(), Unsupported> {
    let mover = walk
        .mover
        .ok_or(Unsupported("a walker without movement state"))?;
    if mover.paths.is_empty() {
        return Err(Unsupported("the path request (FUN_14114e410)"));
    }
    let mut plan = walk.plan;
    match mover.motion(&walk.me, walk.dt, walk.world, plan.as_mut())? {
        Motion::Moving => Ok(()),
        Motion::Arrived => Err(Unsupported("a MoveTo arrives (set_state 1)")),
        Motion::Done => Err(Unsupported("a MoveTo whose unit cannot move on")),
    }
}

/// A MoveTo as a unit's own top action (a wandering animal's).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MoveToAction {
    pub state: MoveTo,
    /// The point it walks to (`+0x38`..`+0x40`), when it has no target object.
    pub goal: Option<[f32; 3]>,
    /// `+0x68`.
    pub range: f32,
}

impl MoveToAction {
    pub fn update(&mut self, walk: Walk<'_>) -> Result<(), Unsupported> {
        match self.state {
            MoveTo::Moving => walk_on(walk),
            MoveTo::Start => {
                let goal = self
                    .goal
                    .ok_or(Unsupported("a MoveTo of its own to a target object"))?;
                self.state = MoveTo::Moving;
                start_walk(walk, Goal::Point(goal), self.range)
            }
            MoveTo::Arrived => Err(Unsupported("a MoveTo that arrived (its removal)")),
        }
    }
}
