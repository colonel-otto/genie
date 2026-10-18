//! The MoveTo action (`RGE_Action_Move_To`, type 1, vtable `0x142ed6030`): a unit walks to a target
//! object or a point. Builders, birds and wandering animals run it, as a sub-action or on its own.
//!
//! 158041: start (slot 5) `0x140aa2100`, update (slot 6) `0x140aa2a70`, set_state (slot 16)
//! `0x140aa29e0`. A MoveTo in state 3 asks for a path (`FUN_14114e410`) and turns to state 4; in
//! state 4 each update runs the motion step (object slot 193, [`Mover::motion`]).

use crate::movement::{Me, Motion, Mover, Planning, footprint_gap, footprint_gap_squared};
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
    /// 2: the motion step gave up and the MoveTo waits before it asks again (its timer `+0x70`
    /// up to `+0x74`, the owner's [`MoveWait`]).
    Waiting,
    /// 13: it failed; its update returns 3 (with notify 0x1f9 unless it is quiet).
    Failed,
}

/// A MoveTo's wait after the motion step gave up (`+0x6c` `movement_wait_state`: 0 never, 1
/// waiting, 2 asked again; the timer `+0x70` and its end `+0x74`), held by the action that owns
/// the MoveTo; a new MoveTo starts without one.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MoveWait {
    pub state: u8,
    pub timer: f32,
    pub delay: f32,
}

impl MoveTo {
    /// The state's number, which word 9 adds.
    pub fn number(self) -> u8 {
        match self {
            MoveTo::Arrived => 1,
            MoveTo::Start => 3,
            MoveTo::Moving => 4,
            MoveTo::Waiting => 2,
            MoveTo::Failed => 13,
        }
    }
}

/// What a walking unit's update needs: its movement state, where it is, and the world; the
/// planners and the world's time for a path request (None where the caller has none).
pub struct Walk<'a> {
    pub mover: Option<&'a mut Mover>,
    pub me: Me,
    pub dt: f32,
    /// The objects' places; the motion step moves the unit onto a waypoint it reached.
    pub world: &'a mut Spatial,
    pub plan: Option<Planning<'a>>,
}

impl Walk<'_> {
    /// The same walk for a call that ends before this one is used again.
    pub fn reborrow(&mut self) -> Walk<'_> {
        Walk {
            mover: self.mover.as_deref_mut(),
            me: self.me,
            dt: self.dt,
            world: &mut *self.world,
            plan: self.plan.as_mut().map(|p| p.reborrow()),
        }
    }
}

/// Where a MoveTo goes: a target object or a point (`x, y, z` on the map).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Goal {
    Target(i32),
    Point([f32; 3]),
}

/// A starting MoveTo's update (state 3): unless the unit is within `range` of its goal already, it
/// asks for a path (`FUN_14114e410`) and turns to state 4, whose set_state runs the update again at
/// once ([`walk_on`]); returns the MoveTo's state after it. In range is the footprints' squared
/// gap (`FUN_14118f8c0`) to a target object, the gap from the unit's footprint to a point, at most
/// `range` squared. A unit already in range ends the MoveTo (set_state 1, code 0) when it has a
/// unit AI not on order 711 or 733; `quiet` says the caller's MoveTo is quiet (its `+0x60` set, so
/// set_state 1 keeps the sprite and `+0x7b8`) and the unit's AI qualifies. Any other unit in range
/// is not modelled.
pub fn start_walk(
    walk: Walk<'_>,
    goal: Goal,
    range: f32,
    quiet: bool,
) -> Result<MoveTo, Unsupported> {
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
    let squared = if target == -1 {
        let gap = footprint_gap(walk.me.position, ground, point, radius);
        gap * gap
    } else {
        footprint_gap_squared(walk.me.position, ground, point, radius)
    };
    if squared <= range * range {
        if quiet {
            return Ok(MoveTo::Arrived);
        }
        return Err(Unsupported(
            "a MoveTo in range when it starts (set_state 1)",
        ));
    }
    let now = walk.plan.as_ref().map_or(0, |p| p.now);
    mover.request_path(walk.me.position, point, range, target, now);
    // the MoveTo's point (`+0x38`): the target's place when it asks for the path
    mover.move_to_point = (target != -1).then_some(point);
    walk_on(Walk {
        mover: Some(mover),
        ..walk
    })
}

/// A moving MoveTo's update (state 4, `0x140aa2a70` from `LAB_140aa320c`): the motion step
/// (slot 193); returns the MoveTo's state after it. A unit that arrived ends the MoveTo
/// (set_state 1, `0x140aa29e0`, which also sets the unit's idle sprite and clears its `+0x7b8`,
/// and `FUN_14114d810` its facet; the slice holds neither). A unit that cannot go on (set_state 2
/// or 13) and a MoveTo without its path are not modelled.
pub fn walk_on(walk: Walk<'_>) -> Result<MoveTo, Unsupported> {
    walk_or_give_up(walk)?.ok_or(Unsupported("a MoveTo whose unit cannot move on"))
}

/// [`walk_on`] for a MoveTo that handles the motion step giving up (it returns 1): None then.
pub fn walk_or_give_up(walk: Walk<'_>) -> Result<Option<MoveTo>, Unsupported> {
    let mover = walk
        .mover
        .ok_or(Unsupported("a walker without movement state"))?;
    if mover.paths.is_empty() {
        return Err(Unsupported("the path request (FUN_14114e410)"));
    }
    let mut plan = walk.plan;
    let mut me = walk.me;
    Ok(
        match mover.motion(&mut me, walk.dt, walk.world, plan.as_mut())? {
            Motion::Moving => Some(MoveTo::Moving),
            Motion::Arrived => Some(MoveTo::Arrived),
            Motion::Done => None,
        },
    )
}

/// A target leaves the MoveTo's point when it is more than this off it on either axis
/// (`DAT_142eb6e58`, `DAT_142e7f384`).
const OFF_LOW: f32 = crate::image::move_to::OFF_LOW;
const OFF_HIGH: f32 = crate::image::move_to::OFF_HIGH;

/// The moving MoveTo's check of its target object (`0x140aa2a70` in state 4, before the motion
/// step): a target that left the MoveTo's point (more than half a tile off on either axis) makes
/// a MoveTo with `+0x61` at 1 (`quiet`: a hunt's and its Attack's) ask for a path again: it turns
/// to state 3 (the slice's [`MoveTo::Start`]) and its update ends there. With `+0x61` at 0 (a
/// MoveTo of its own, FOLLOW's) the MoveTo moves its path's end to the target when the unit walks
/// to that end ([`Mover::walks_to_path_end`]) and the straight line to the target is clear
/// (`FUN_141052b90`, `FUN_141052350`; then `FUN_140f9b890` on both paths), which is not modelled;
/// else it tells the unit (notify 0x1fc), turns to state 3 and the motion step runs on. True when
/// the MoveTo turns to state 3.
pub fn target_moved(walk: &Walk<'_>, target: i32, quiet: bool) -> Result<bool, Unsupported> {
    let t = walk
        .world
        .get_known(target)?
        .ok_or(Unsupported("a MoveTo whose target is gone"))?;
    let point = walk
        .mover
        .as_deref()
        .ok_or(Unsupported("a walker without movement state"))?
        .move_to_point
        .ok_or(Unsupported(
            "a MoveTo to a target whose point the slice does not hold",
        ))?;
    let (dx, dy) = (point[0] - t.position[0], point[1] - t.position[1]);
    if !(dx < OFF_LOW || OFF_HIGH < dx || dy < OFF_LOW || OFF_HIGH < dy) {
        return Ok(false);
    }
    if !quiet
        && walk
            .mover
            .as_deref()
            .ok_or(Unsupported("a walker without movement state"))?
            .walks_to_path_end()?
    {
        return Err(Unsupported(
            "a MoveTo whose path's end follows its target (FUN_140f9b890 after the line test)",
        ));
    }
    Ok(true)
}

/// A MoveTo as a unit's own top action (a wandering animal's, a moved or following unit's). Its
/// `+0x60` and `+0x61` are 0: it is not quiet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MoveToAction {
    pub state: MoveTo,
    /// The point it walks to (`+0x38`..`+0x40`), when it has no target object.
    pub goal: Option<[f32; 3]>,
    /// The target object it walks to (`+0x20`, its id at `+0x30`), −1 for a point; its point is
    /// the mover's [`Mover::move_to_point`].
    pub target: i32,
    /// `+0x68`.
    pub range: f32,
    /// It shows the master's running sprite (an AI move that does not walk, slot 0x340 with
    /// its seventh argument 0), else its walking one.
    pub run: bool,
}

/// A MoveTo's start counts the unit at its target within this gap (`0x140aa2100`,
/// `DAT_142eb99cc`).
const ARRIVED: f32 = crate::image::move_to::ARRIVED;

/// What one update of a [`MoveToAction`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MoveStep {
    /// It is done (its update in state 1 returns 1): the action list removes it.
    pub done: bool,
    /// Its target left its point: it told the unit (notify 0x1fc) and turned to state 3.
    pub target_moved: bool,
}

impl MoveToAction {
    /// A new MoveTo of its own to the object `target` within `range` (`FUN_140aa0df0`; its point
    /// the target's place, which the slice reads when it asks for the path), showing the master's
    /// running sprite (`run`) or its walking one, as its start (slot 0x808 -> `0x140aa2100`)
    /// leaves it: a unit more than [`ARRIVED`] from the target's footprint (`FUN_14118f680`) in
    /// state 3 with speed (slot 0x208), 13 without. A unit at the target already (set_state 1:
    /// the standing sprite, `+0x7b8` cleared) is not modelled. `me` is the unit's place.
    pub fn to_target(
        world: &Spatial,
        me: [f32; 3],
        mover: &Mover,
        target: i32,
        range: f32,
        run: bool,
    ) -> Result<Self, Unsupported> {
        let t = world
            .get_known(target)?
            .ok_or(Unsupported("a MoveTo to an object the world lacks"))?;
        let ground = [mover.stats.radius[0], mover.stats.radius[1]];
        if footprint_gap(me, ground, t.position, t.radius) <= ARRIVED {
            return Err(Unsupported("a MoveTo to an object that starts arrived"));
        }
        Ok(MoveToAction {
            state: if mover.stats.speed != 0.0 {
                MoveTo::Start
            } else {
                MoveTo::Failed
            },
            goal: None,
            target,
            range,
            run,
        })
    }

    /// A MoveTo of its own is not quiet: its set_state 1 or 13 shows the standing sprite
    /// ([`Mover::shows_standing`]).
    fn ended(&self, walk: &mut Walk<'_>) {
        if matches!(self.state, MoveTo::Arrived | MoveTo::Failed)
            && let Some(m) = walk.mover.as_deref_mut()
        {
            m.shows_standing();
        }
    }

    /// One update (`0x140aa2240`). A target that is gone (the update's first check: notify 0x1fb,
    /// code 4) is not modelled.
    pub fn update(&mut self, mut walk: Walk<'_>) -> Result<MoveStep, Unsupported> {
        let mut step = MoveStep::default();
        if self.target != -1 && walk.world.get_known(self.target)?.is_none() {
            return Err(Unsupported("a MoveTo of its own whose target is gone"));
        }
        match self.state {
            MoveTo::Moving => {
                if self.target != -1 && target_moved(&walk, self.target, false)? {
                    // set_state 3, then the motion step (`LAB_140aa320c`): moving on, the MoveTo
                    // stays in state 3
                    step.target_moved = true;
                    self.state = match walk_on(walk.reborrow())? {
                        MoveTo::Moving => MoveTo::Start,
                        other => other,
                    };
                } else {
                    self.state = walk_on(walk.reborrow())?;
                }
                self.ended(&mut walk);
                Ok(step)
            }
            MoveTo::Start => {
                let goal = match (self.target, self.goal) {
                    (-1, Some(point)) => Goal::Point(point),
                    (-1, None) => return Err(Unsupported("a MoveTo of its own without a goal")),
                    (id, _) => Goal::Target(id),
                };
                // its sprite, the running or the walking one, sets the unit's speed
                if let Some(m) = walk.mover.as_deref_mut() {
                    m.stats.speed = if self.run {
                        m.stats.run_speed
                    } else {
                        m.stats.walk_speed
                    };
                }
                self.state = MoveTo::Moving;
                self.state = start_walk(walk.reborrow(), goal, self.range, false)?;
                self.ended(&mut walk);
                Ok(step)
            }
            MoveTo::Arrived => {
                step.done = true;
                Ok(step)
            }
            MoveTo::Waiting => Err(Unsupported("a waiting MoveTo of its own (state 2)")),
            MoveTo::Failed => Err(Unsupported("a failed MoveTo of its own (state 13)")),
        }
    }
}
