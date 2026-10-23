//! The `MoveTo` action (`RGE_Action_Move_To`, type 1, vtable `0x142ed6030`): a unit walks to a target
//! object or a point. Builders, birds and wandering animals run it, as a sub-action or on its own.
//!
//! 158041: start (slot 5) `0x140aa2100`, update (slot 6) `0x140aa2a70`, `set_state` (slot 16)
//! `0x140aa29e0`. A `MoveTo` in state 3 asks for a path (`FUN_14114e410`) and turns to state 4; in
//! state 4 each update runs the motion step (object slot 193, [`Mover::motion`]).

use crate::GenieError;
use crate::movement::{Me, Motion, Mover, PathAsk, Planning, footprint_gap, footprint_gap_squared};
use crate::spatial::Spatial;

/// The `MoveTo` action's type, which word 9 adds when it is a unit's top action.
pub const MOVE_TO: i16 = 1;

/// The `MoveTo`'s state (`+0x18`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveTo {
    /// 1: at the target when it started.
    Arrived,
    /// 3: it starts to move (`0x140aa2100`).
    Start,
    /// 4: its first update asked for a path; the unit moves from the next update on.
    Moving,
    /// 2: the motion step gave up and the `MoveTo` waits before it asks again (its timer `+0x70`
    /// up to `+0x74`, the owner's [`MoveWait`]).
    Waiting,
    /// 13: it failed; its update returns 3 (with notify 0x1f9 unless it is quiet).
    Failed,
}

/// A `MoveTo`'s wait after the motion step gave up (`+0x6c` `movement_wait_state`: 0 never, 1
/// waiting, 2 asked again; the timer `+0x70` and its end `+0x74`), held by the action that owns
/// the `MoveTo`; a new `MoveTo` starts without one.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MoveWait {
    /// `+0x6c`, `movement_wait_state`: 0 never waited, 1 waiting, 2 asked again.
    pub state: u8,
    /// `+0x70`: the time waited.
    pub timer: f32,
    /// `+0x74`: how long it waits.
    pub delay: f32,
}

impl MoveWait {
    /// The `MoveTo`'s answer to the motion step giving up (it returns 1, `0x140aa2a70` in state
    /// 4): a human player's villager (`waits`) whose `MoveTo` has not waited yet (`+0x6c` 0) waits
    /// (`set_state` 2, `+0x6c` 1) a second with a stance (`+0x1a9`) below 2, else two, then asks
    /// again; any other `MoveTo` fails (`set_state` 13). Returns the `MoveTo`'s state.
    pub const fn give_up(&mut self, waits: bool, stance: u8) -> MoveTo {
        if self.state != 0 || !waits {
            return MoveTo::Failed;
        }
        self.delay = if stance < 2 { 1.0 } else { 2.0 };
        self.state = 1;
        MoveTo::Waiting
    }
}

impl MoveTo {
    /// The state's number, which word 9 adds.
    #[must_use]
    pub const fn number(self) -> u8 {
        match self {
            Self::Arrived => 1,
            Self::Start => 3,
            Self::Moving => 4,
            Self::Waiting => 2,
            Self::Failed => 13,
        }
    }
}

/// What a walking unit's update needs: its movement state, where it is, and the world; the
/// planners and the world's time for a path request (None where the caller has none).
#[derive(Debug)]
pub struct Walk<'a> {
    /// The unit's movement state, if it moves.
    pub mover: Option<&'a mut Mover>,
    /// The unit.
    pub me: Me,
    /// The world's dt in seconds.
    pub dt: f32,
    /// The objects' places; the motion step moves the unit onto a waypoint it reached.
    pub world: &'a mut Spatial,
    /// The planners and the world's time for a path request, None where the caller has none.
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

/// Where a `MoveTo` goes: a target object or a point (`x, y, z` on the map).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Goal {
    /// A target object.
    Target(i32),
    /// A point.
    Point([f32; 3]),
}

/// A starting `MoveTo`'s update (state 3): unless the unit is within `range` of its goal already, it
/// asks for a path (`FUN_14114e410`) and turns to state 4, whose `set_state` runs the update again at
/// once ([`walk_on`]); returns the `MoveTo`'s state after it. In range is the footprints' squared
/// gap (`FUN_14118f8c0`) to a target object, the gap from the unit's footprint to a point, at most
/// `range` squared. A unit already in range ends the `MoveTo` (`set_state` 1, code 0) when it has a
/// unit AI not on order 711 or 733 ([`start_walk_asked`]; here it does); a `MoveTo` that is not
/// `quiet` (its `+0x60` clear) then shows the unit's idle sprite (slot 0x68 with slot 0x798's: the
/// master's standing sprite, [`Mover::shows_standing`]; the sprite itself and `+0x7b8`, cleared,
/// are the caller's).
pub fn start_walk(
    walk: Walk<'_>,
    goal: Goal,
    range: f32,
    quiet: bool,
) -> Result<MoveTo, GenieError> {
    start_walk_or_give_up(walk, goal, range, quiet)?.ok_or(GenieError::unsupported(
        "a MoveTo whose unit cannot move on",
    ))
}

/// [`start_walk`] for a `MoveTo` that handles the motion step giving up in the update its
/// `set_state(4)` runs: None then (the `MoveTo` is in state 4, its sprite shown; the owner's
/// answer, [`walk_or_give_up`]'s, decides what follows).
pub fn start_walk_or_give_up(
    walk: Walk<'_>,
    goal: Goal,
    range: f32,
    quiet: bool,
) -> Result<Option<MoveTo>, GenieError> {
    start_walk_asked(walk, goal, range, quiet, PathAsk::default())
}

/// [`start_walk_or_give_up`] with what the unit's AI says (`0x140aa2a70` in state 3,
/// [`PathAsk`]): a unit in range ends the `MoveTo` only when [`PathAsk::ends_in_range`], and the
/// path request takes the rest ([`Mover::request_path_asked`]).
pub fn start_walk_asked(
    walk: Walk<'_>,
    goal: Goal,
    range: f32,
    quiet: bool,
    ask: PathAsk,
) -> Result<Option<MoveTo>, GenieError> {
    let (point, target, radius) = match goal {
        Goal::Target(id) => {
            let t = walk
                .world
                .get_known(id)?
                .ok_or(GenieError::unsupported("a MoveTo whose target is gone"))?;
            (t.position, id, t.radius)
        }
        Goal::Point(p) => (p, -1, [0.0, 0.0]),
    };
    let mover = walk
        .mover
        .ok_or(GenieError::unsupported("a walker without movement state"))?;
    let ground = [mover.stats.radius[0], mover.stats.radius[1]];
    let squared = if target == -1 {
        let gap = footprint_gap(walk.me.position, ground, point, radius);
        gap * gap
    } else {
        footprint_gap_squared(walk.me.position, ground, point, radius)
    };
    if ask.ends_in_range && squared <= range * range {
        if !quiet {
            mover.shows_standing();
        }
        return Ok(Some(MoveTo::Arrived));
    }
    let now = walk.plan.as_ref().map_or(0, |p| p.now);
    mover.request_path_asked(walk.me.position, point, range, target, now, ask);
    // the MoveTo's point (`+0x38`): the target's place when it asks for the path
    mover.move_to_point = (target != -1).then_some(point);
    walk_or_give_up(Walk {
        mover: Some(mover),
        ..walk
    })
}

/// A moving `MoveTo`'s update (state 4, `0x140aa2a70` from `LAB_140aa320c`): the motion step
/// (slot 193); returns the `MoveTo`'s state after it. A unit that arrived ends the `MoveTo`
/// (`set_state` 1, `0x140aa29e0`, which also sets the unit's idle sprite and clears its `+0x7b8`,
/// and `FUN_14114d810` its facet; the slice holds neither). A unit that cannot go on (`set_state` 2
/// or 13) and a `MoveTo` without its path are not modelled.
pub fn walk_on(walk: Walk<'_>) -> Result<MoveTo, GenieError> {
    walk_or_give_up(walk)?.ok_or(GenieError::unsupported(
        "a MoveTo whose unit cannot move on",
    ))
}

/// [`walk_on`] for a `MoveTo` that handles the motion step giving up (it returns 1): None then.
pub fn walk_or_give_up(walk: Walk<'_>) -> Result<Option<MoveTo>, GenieError> {
    let mover = walk
        .mover
        .ok_or(GenieError::unsupported("a walker without movement state"))?;
    if mover.paths.is_empty() {
        return Err(GenieError::unsupported("the path request (FUN_14114e410)"));
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

/// A target leaves the `MoveTo`'s point when it is more than this off it on either axis
/// (`DAT_142eb6e58`, `DAT_142e7f384`).
const OFF_LOW: f32 = crate::image::move_to::OFF_LOW;
const OFF_HIGH: f32 = crate::image::move_to::OFF_HIGH;

/// What [`target_moved`] found.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Follow {
    /// The target stayed on the `MoveTo`'s point.
    Stays,
    /// The `MoveTo` turns to state 3: it asks for a path again.
    Again,
    /// The path's end follows the target to this point ([`follow_path_end`]); the walk goes on.
    PathEnd([f32; 3]),
}

impl Follow {
    /// The `MoveTo` turns to state 3.
    #[must_use]
    pub const fn again(self) -> bool {
        matches!(self, Self::Again)
    }
}

/// [`Follow::PathEnd`]: the unit's paths end at `point` ([`Mover::follow_path_end`]) and the
/// `MoveTo`'s point becomes it.
///
/// # Errors
///
/// A walker without movement state.
pub fn follow_path_end(walk: &mut Walk<'_>, point: [f32; 3]) -> Result<(), GenieError> {
    let mover = walk
        .mover
        .as_deref_mut()
        .ok_or(GenieError::unsupported("a walker without movement state"))?;
    mover.follow_path_end(point);
    mover.move_to_point = Some(point);
    Ok(())
}

/// The moving `MoveTo`'s check of its target object (`0x140aa2a70` in state 4, before the motion
/// step): a target that left the `MoveTo`'s point (more than half a tile off on either axis) makes
/// a `MoveTo` with `+0x61` at 1 (`quiet`: a hunt's and its Attack's) ask for a path again: it turns
/// to state 3 (the slice's [`MoveTo::Start`]) and its update ends there. With `+0x61` at 0 (a
/// `MoveTo` of its own, FOLLOW's, a unit's own Attack's) the `MoveTo` moves its path's end to the
/// target ([`Mover::follow_path_end`], its point the target's place) when the unit walks to that
/// end ([`Mover::walks_to_path_end`]) and the straight line from the unit to the target is clear
/// (`FUN_141052b90`, then `FUN_141052350` with flags `0x100`, [`Spatial::segment_blocked`]: the
/// obstruction manager's buffer radius and ignore list, which nothing set, none), and walks on;
/// else it tells the unit (notify 0x1fc), turns to state 3 and the motion step runs on. True when
/// the `MoveTo` turns to state 3.
pub fn target_moved(walk: &Walk<'_>, target: i32, quiet: bool) -> Result<Follow, GenieError> {
    let t = walk
        .world
        .get_known(target)?
        .ok_or(GenieError::unsupported("a MoveTo whose target is gone"))?;
    let point = walk
        .mover
        .as_deref()
        .ok_or(GenieError::unsupported("a walker without movement state"))?
        .move_to_point
        .ok_or(GenieError::unsupported(
            "a MoveTo to a target whose point the slice does not hold",
        ))?;
    let (dx, dy) = (point[0] - t.position[0], point[1] - t.position[1]);
    if !(dx < OFF_LOW || OFF_HIGH < dx || dy < OFF_LOW || OFF_HIGH < dy) {
        return Ok(Follow::Stays);
    }
    if quiet {
        return Ok(Follow::Again);
    }
    let mover = walk
        .mover
        .as_deref()
        .ok_or(GenieError::unsupported("a walker without movement state"))?;
    if mover.walks_to_path_end()? {
        let me = walk
            .world
            .get(walk.me.id)
            .map_or(walk.me.position, |p| p.position);
        let (a, b) = ([me[0], me[1]], [t.position[0], t.position[1]]);
        walk.world.check_known_records([
            a[0].min(b[0]),
            a[1].min(b[1]),
            a[0].max(b[0]),
            a[1].max(b[1]),
        ])?;
        if !walk
            .world
            .segment_blocked(a, b, 0.0, mover.stats.restriction_mask, &[])
        {
            return Ok(Follow::PathEnd(t.position));
        }
    }
    Ok(Follow::Again)
}

/// A `MoveTo` as a unit's own top action (a wandering animal's, a moved or following unit's). Its
/// `+0x60` and `+0x61` are 0: it is not quiet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MoveToAction {
    /// Its state (`+0x18`).
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
    /// Its wait after the motion step gave up.
    pub wait: MoveWait,
}

/// A `MoveTo`'s start counts the unit at its target within this gap (`0x140aa2100`,
/// `DAT_142eb99cc`).
const ARRIVED: f32 = crate::image::move_to::ARRIVED;
/// A `MoveTo` to an object the world does not hold.
pub(crate) const TARGET_LACKING: GenieError =
    GenieError::unsupported("a MoveTo to an object the world lacks");
/// Livestock's `MoveTo` to a town centre walks to its place less this on both axes
/// (`FUN_140aa0df0`, `DAT_142eae8d0`).
const TOWN_CENTRE_OFFSET: f32 = crate::image::artifact::TOWN_CENTRE_OFFSET;
/// The town centre's master (`FUN_140aa0df0`'s `0x6109` raw), to which livestock walks beside.
const TOWN_CENTRE: i16 = 109;

/// `FUN_140aa0df0`'s test of a `MoveTo`'s target object of master `master` in `state`:
/// livestock (`livestock`, the unit's class 58) moving to a town centre (master 109) holds none,
/// and no `MoveTo` holds a target in state 7 or later (`FUN_140a7a230`); any other target is
/// held, and a `MoveTo` of its own (not quiet) takes it (slot 0x178).
#[must_use]
pub const fn holds_object(livestock: bool, master: i16, state: u8) -> bool {
    !(livestock && master == TOWN_CENTRE) && state < 7
}

/// What one update of a [`MoveToAction`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MoveStep {
    /// It is done (its update in state 1 returns 1): the action list removes it.
    pub done: bool,
    /// Its target left its point: it told the unit (notify 0x1fc) and turned to state 3.
    pub target_moved: bool,
    /// The motion step could not go on (its result 1): the caller sets state 13, or for a
    /// human's villager the wait of state 2 (`LAB_140aa320c`).
    pub gave_up: bool,
    /// It had failed (state 13): its update returns 3 after the notify 0x1f9 (an own `MoveTo` is
    /// never quiet); the action list removes it.
    pub failed: bool,
}

impl MoveToAction {
    /// A new `MoveTo` of its own to the object `target` within `range` (`FUN_140aa0df0`; its point
    /// `+0x38` the target's place, [`Mover::move_to_point`], which a path request renews), showing
    /// the master's running sprite (`run`) or its walking one, as its start (slot 0x808 ->
    /// `0x140aa2100`) leaves it: a unit more than [`ARRIVED`] from the target's footprint
    /// (`FUN_14118f680`) in state 3 with speed (slot 0x208), 13 without; a unit at the target
    /// already in state 1, which its next update ends. States 1 and 13 (`0x140aa29e0`, not quiet)
    /// show the master's standing sprite ([`Mover::shows_standing`]; the sprite itself and
    /// `+0x7b8`, cleared, are the caller's, [`MoveToAction::shows_standing`]). `me` is the unit's
    /// place.
    pub fn to_target(
        world: &Spatial,
        me: [f32; 3],
        mover: &mut Mover,
        target: i32,
        range: f32,
        run: bool,
    ) -> Result<Self, GenieError> {
        let t = world.get_known(target)?.ok_or(TARGET_LACKING)?;
        let ground = [mover.stats.radius[0], mover.stats.radius[1]];
        mover.move_to_point = Some(t.position);
        let state = if footprint_gap(me, ground, t.position, t.radius) <= ARRIVED {
            MoveTo::Arrived
        } else if mover.stats.speed == 0.0 {
            MoveTo::Failed
        } else {
            MoveTo::Start
        };
        if state != MoveTo::Start {
            mover.shows_standing();
        }
        Ok(Self {
            state,
            goal: None,
            target,
            range,
            run,
            wait: MoveWait::default(),
        })
    }

    /// A new `MoveTo` of its own (quiet, `+0x60`, 0) to the object `target` within `range`
    /// (`FUN_140aa0df0`), then its start (slot 0x808 -> `0x140aa2100`). Livestock (`livestock`,
    /// the unit's class 58) moving to a town centre (master 109) holds no target and walks to
    /// the centre's place less [`TOWN_CENTRE_OFFSET`] on both axes; a target in a `state` of 7
    /// or later is not held either (`FUN_140a7a230`: its id stays −1) and the `MoveTo` walks to
    /// its place, as a `MoveTo` to that point starts ([`MoveToAction::to_point`]). Any other
    /// target is held ([`MoveToAction::to_target`]), and the caller takes it (slot
    /// 0x178, [`MoveToAction::target`] not −1).
    ///
    /// # Errors
    ///
    /// A target the world lacks.
    pub fn to_object(
        world: &Spatial,
        me: [f32; 3],
        mover: &mut Mover,
        (target, state): (i32, u8),
        range: f32,
        (run, livestock): (bool, bool),
    ) -> Result<Self, GenieError> {
        let t = world.get_known(target)?.ok_or(TARGET_LACKING)?;
        let at = t.position;
        let goal = if livestock && t.master == TOWN_CENTRE {
            [
                at[0] - TOWN_CENTRE_OFFSET,
                at[1] - TOWN_CENTRE_OFFSET,
                at[2],
            ]
        } else if holds_object(livestock, t.master, state) {
            return Self::to_target(world, me, mover, target, range, run);
        } else {
            at
        };
        Ok(Self::to_point(goal, range, run, Some(mover)))
    }

    /// A new `MoveTo` of its own (not quiet) to `point` within `range` (`FUN_140aa0d30`), showing
    /// the master's running sprite (`run`) or its walking one, as its start (slot 0x808 ->
    /// `0x140aa2100`) leaves it. Without a target object the start's gap (`FUN_14118f680` with
    /// none) is `f32::MAX`, never within 0.01: a unit with speed (slot 0x208, `UCOMISS`: a NaN
    /// speed has some) is in state 3, one without in 13 (`0x140aa29e0`, not quiet: the master's
    /// standing sprite, [`Mover::shows_standing`]; the sprite itself and `+0x7b8`, cleared, are the
    /// caller's, [`MoveToAction::shows_standing`]). A unit without movement state has no speed.
    #[must_use]
    pub fn to_point(point: [f32; 3], range: f32, run: bool, mover: Option<&mut Mover>) -> Self {
        let state = match mover {
            Some(m) if m.stats.speed != 0.0 => MoveTo::Start,
            Some(m) => {
                m.shows_standing();
                MoveTo::Failed
            }
            None => MoveTo::Failed,
        };
        Self {
            state,
            goal: Some(point),
            target: -1,
            range,
            run,
            wait: MoveWait::default(),
        }
    }

    /// It ended at its start, the unit at its target (state 1).
    #[must_use]
    pub fn arrived(&self) -> bool {
        self.state == MoveTo::Arrived
    }

    /// Its start ended it (state 1 at its target, or 13 for a unit without speed), whose
    /// `set_state` (`0x140aa29e0`, not quiet) shows the master's standing sprite and clears the
    /// unit's `+0x7b8`: the caller's ([`crate::turn::Unit::shows_standing_sprite`]).
    #[must_use]
    pub const fn shows_standing(&self) -> bool {
        matches!(self.state, MoveTo::Arrived | MoveTo::Failed)
    }

    /// The point it walks to (`+0x38`..`+0x40`): its own, or for a target object the mover's
    /// [`Mover::move_to_point`] (the target's place when it was made or last asked for a path).
    #[must_use]
    pub fn point(&self, mover: Option<&Mover>) -> Option<[f32; 3]> {
        self.goal.or_else(|| mover.and_then(|m| m.move_to_point))
    }

    /// A `MoveTo` of its own is not quiet: its `set_state` 1 or 13 shows the standing sprite
    /// ([`Mover::shows_standing`]).
    fn ended(&self, walk: &mut Walk<'_>) {
        if matches!(self.state, MoveTo::Arrived | MoveTo::Failed)
            && let Some(m) = walk.mover.as_deref_mut()
        {
            m.shows_standing();
        }
    }

    /// One update (`0x140aa2240`), its state 3 with what the unit's AI says (`ask`,
    /// [`start_walk_asked`]). A target that is gone (the update's first check: notify 0x1fb,
    /// code 4) is not modelled.
    pub fn update(&mut self, mut walk: Walk<'_>, ask: PathAsk) -> Result<MoveStep, GenieError> {
        let mut step = MoveStep::default();
        if self.target != -1 && walk.world.get_known(self.target)?.is_none() {
            return Err(GenieError::unsupported(
                "a MoveTo of its own whose target is gone",
            ));
        }
        match self.state {
            MoveTo::Moving => {
                let follow = if self.target == -1 {
                    Follow::Stays
                } else {
                    target_moved(&walk, self.target, false)?
                };
                if let Follow::PathEnd(p) = follow {
                    follow_path_end(&mut walk, p)?;
                }
                if follow.again() {
                    // set_state 3, then the motion step (`LAB_140aa320c`): moving on, the MoveTo
                    // stays in state 3
                    step.target_moved = true;
                    self.state = match walk_on(walk.reborrow())? {
                        MoveTo::Moving => MoveTo::Start,
                        other => other,
                    };
                } else {
                    match walk_or_give_up(walk.reborrow())? {
                        Some(state) => self.state = state,
                        None => step.gave_up = true,
                    }
                }
                self.ended(&mut walk);
            }
            MoveTo::Start => {
                // case 3 after a wait: `+0x6c` goes from 1 to 2 as it asks again
                if self.wait.state == 1 {
                    self.wait.state = 2;
                }
                let goal = match (self.target, self.goal) {
                    (-1, Some(point)) => Goal::Point(point),
                    (-1, None) => {
                        return Err(GenieError::unsupported(
                            "a MoveTo of its own without a goal",
                        ));
                    }
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
                match start_walk_asked(walk.reborrow(), goal, self.range, false, ask)? {
                    Some(state) => self.state = state,
                    None => step.gave_up = true,
                }
                self.ended(&mut walk);
            }
            MoveTo::Arrived => step.done = true,
            // case 2: the timer runs by dt; at its end the MoveTo asks again (set_state 3)
            MoveTo::Waiting => {
                self.wait.timer += walk.dt;
                if self.wait.delay <= self.wait.timer {
                    self.wait.timer = 0.0;
                    self.state = MoveTo::Start;
                }
            }
            // 0x140aa2a70's state 13: the notify (the caller's), then code 3
            MoveTo::Failed => step.failed = true,
        }
        Ok(step)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spatial::Placed;

    /// The 4v4's woodcutter 31101 (a human player's villager, stance 0) whose Attack's `MoveTo`
    /// the motion step gave up on in update 5101: it waits a second, `+0x6c` 1; once it waited it
    /// fails, as a computer player's or another class's at once. A stance of 2 waits two.
    #[test]
    fn a_human_villager_s_moveto_waits_once_before_it_fails() {
        let mut wait = MoveWait::default();
        assert_eq!(wait.give_up(true, 0), MoveTo::Waiting);
        assert_eq!((wait.state, wait.delay), (1, 1.0));
        assert_eq!(wait.give_up(true, 0), MoveTo::Failed);
        assert_eq!(MoveWait::default().give_up(false, 0), MoveTo::Failed);
        let mut guarded = MoveWait::default();
        assert_eq!(guarded.give_up(true, 2), MoveTo::Waiting);
        assert_eq!(guarded.delay, 2.0);
    }

    fn sheep(x: f32, y: f32) -> Placed {
        Placed {
            position: [x, y, 0.0],
            radius: [0.2, 0.2],
            class: 10,
            player: 1,
            master: 594,
            obstruction_type: 0,
            buildable_on: 0,
            tall: true,
            speed: 0.0,
            unknown: None,
            obstruction: None,
        }
    }

    #[test]
    fn a_move_to_an_object_at_hand_starts_arrived_and_shows_the_standing_sprite() {
        let mut world = Spatial::new([120, 120]);
        world.insert(9185, sheep(30.4, 40.0));
        world.insert(9186, sheep(35.0, 40.0));
        let mut walker = crate::hunt::tests::walker();
        walker.stats.stand_speed = 0.5;
        // footprints 0 apart: within ARRIVED, state 1 with the standing sprite's speed
        let me = [30.0, 40.0, 0.0];
        let at_hand = MoveToAction::to_target(&world, me, &mut walker, 9185, 0.0, false).unwrap();
        assert!(at_hand.arrived());
        assert_eq!(walker.stats.speed, 0.5);
        // 4.6 tiles on it starts walking (state 3)
        let mut walker = crate::hunt::tests::walker();
        let far = MoveToAction::to_target(&world, me, &mut walker, 9186, 0.0, false).unwrap();
        assert!(!far.arrived());
        assert_eq!(walker.stats.speed, 0.8);
    }

    /// `FUN_140aa0df0` on an object: a live target is held (the caller takes it); a target in
    /// state 7 is not (`FUN_140a7a230`) and the `MoveTo` walks to its place; livestock moving to
    /// a town centre walks to its place less 0.2 on both axes without a target. Without a target
    /// the start is state 3, or 13 for a unit without speed.
    #[test]
    fn a_move_to_an_object_holds_it_unless_it_is_gone_or_a_town_centre_for_livestock() {
        let mut world = Spatial::new([120, 120]);
        world.insert(9186, sheep(35.0, 40.0));
        world.insert(
            9149,
            Placed {
                master: 109,
                radius: [2.0, 2.0],
                ..sheep(50.0, 40.0)
            },
        );
        let me = [30.0, 40.0, 0.0];
        let to = |target, livestock, speed: f32| {
            let mut walker = crate::hunt::tests::walker();
            walker.stats.speed = speed;
            MoveToAction::to_object(&world, me, &mut walker, target, 0.0, (false, livestock))
                .unwrap()
        };
        let held = to((9186, 2), false, 0.8);
        assert_eq!(
            (held.target, held.goal, held.state),
            (9186, None, MoveTo::Start)
        );
        let gone = to((9186, 7), false, 0.8);
        assert_eq!(
            (gone.target, gone.goal, gone.state),
            (-1, Some([35.0, 40.0, 0.0]), MoveTo::Start)
        );
        assert_eq!(to((9186, 7), false, 0.0).state, MoveTo::Failed);
        // a villager to the town centre holds it; livestock walks beside it
        assert_eq!(to((9149, 2), false, 0.8).target, 9149);
        let beside = to((9149, 2), true, 0.8);
        assert_eq!(
            (beside.target, beside.goal),
            (-1, Some([50.0 - 0.2, 40.0 - 0.2, 0.0]))
        );
    }

    /// `FUN_140aa0d30` and its start `0x140aa2100`: a point is never within 0.01 (no target, so
    /// `f32::MAX`): state 3 for a unit with speed (a NaN speed has some), 13 for one without,
    /// whose not quiet `set_state` shows the standing sprite (its speed); without movement state
    /// a unit has no speed. Its point is `+0x38`.
    #[test]
    fn a_move_to_a_point_starts_unless_its_unit_has_no_speed() {
        let point = [12.0, 34.0, 1.0];
        let start = |speed: f32| {
            let mut walker = crate::hunt::tests::walker();
            walker.stats.speed = speed;
            walker.stats.stand_speed = 0.25;
            let m = MoveToAction::to_point(point, 0.5, true, Some(&mut walker));
            (m, walker.stats.speed)
        };
        let (moving, speed) = start(0.8);
        assert_eq!(
            (
                moving.state,
                moving.goal,
                moving.target,
                moving.range,
                moving.run
            ),
            (MoveTo::Start, Some(point), -1, 0.5, true)
        );
        assert_eq!(speed, 0.8);
        assert!(!moving.shows_standing());
        assert_eq!(moving.point(None), Some(point));
        assert_eq!(start(f32::NAN).0.state, MoveTo::Start);
        let (stuck, speed) = start(0.0);
        assert_eq!(stuck.state, MoveTo::Failed);
        assert!(stuck.shows_standing());
        assert_eq!(speed, 0.25);
        let none = MoveToAction::to_point(point, 0.0, false, None);
        assert_eq!(none.state, MoveTo::Failed);
    }

    /// `FUN_140aa0df0` puts the target's place in `+0x38` (the mover's `move_to_point`); a unit
    /// without speed away from the target fails at the start (state 13), which shows the
    /// standing sprite as an arrival does.
    #[test]
    fn a_move_to_an_object_holds_its_place_and_a_unit_without_speed_fails_standing() {
        let mut world = Spatial::new([120, 120]);
        world.insert(9186, sheep(35.0, 40.0));
        let me = [30.0, 40.0, 0.0];
        let mut walker = crate::hunt::tests::walker();
        walker.stats.stand_speed = 0.5;
        let far = MoveToAction::to_target(&world, me, &mut walker, 9186, 0.0, false).unwrap();
        assert_eq!(walker.move_to_point, Some([35.0, 40.0, 0.0]));
        assert_eq!(far.point(Some(&walker)), Some([35.0, 40.0, 0.0]));
        assert_eq!(walker.stats.speed, 0.8);
        walker.stats.speed = 0.0;
        let stuck = MoveToAction::to_target(&world, me, &mut walker, 9186, 0.0, false).unwrap();
        assert_eq!(stuck.state, MoveTo::Failed);
        assert!(stuck.shows_standing());
        assert_eq!(walker.stats.speed, 0.5);
    }
}
