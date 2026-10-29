//! The `MoveTo` action (`RGE_Action_Move_To`, type 1, vtable `0x142ed6030`): a unit walks to a target
//! object or a point. Builders, birds and wandering animals run it, as a sub-action or on its own.
//!
//! 158041: start (slot 5) `0x140aa2100`, update (slot 6) `0x140aa2a70`, `set_state` (slot 16)
//! `0x140aa29e0`. A `MoveTo` in state 3 asks for a path (`FUN_14114e410`) and turns to state 4; in
//! state 4 each update runs the motion step (object slot 193, [`Mover::motion`]).

use crate::GenieError;
use crate::map::spatial::Spatial;
use crate::math::rng::Well512;
use crate::movement::{Me, Motion, Planning, footprint_gap, footprint_gap_squared};
use crate::movement::{MoveStats, PathAsk};
use crate::objects::layers::Body;
use crate::objects::sprite::{ActiveSprite, Graphic, new_sprite};
use genie_diff::Diff;

mod action;

pub(crate) use crate::movement::move_to::action::TARGET_LACKING;
pub use crate::movement::move_to::action::{
    MoveDefaults, MoveStep, MoveToAction, Shows, ai_sprite, holds_object, object_range,
    object_sprite, point_sprite,
};

/// The `MoveTo` action's type, which word 9 adds when it is a unit's top action.
pub const MOVE_TO: i16 = 1;

/// The `MoveTo`'s state (`+0x18`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Diff)]
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
/// the `MoveTo`; a new `MoveTo` starts without one ([`MoveWait::default`]).
#[derive(Clone, Copy, Debug, PartialEq, Diff)]
pub struct MoveWait {
    /// `+0x6c`, `movement_wait_state`: 0 never waited, 1 waiting, 2 asked again.
    pub state: u8,
    /// `+0x70`: the time waited.
    pub timer: f32,
    /// `+0x74`: how long it waits.
    pub delay: f32,
}

/// `kDefaultWaitTime`, the wait a `MoveTo`'s constructors set (`+0x74`, `FUN_140aa0d30`,
/// `FUN_140aa0df0`): one second.
const DEFAULT_WAIT: f32 = 1.0;

impl Default for MoveWait {
    fn default() -> Self {
        Self::NEW
    }
}

impl MoveWait {
    /// A new `MoveTo`'s: never waited, no time waited, [`DEFAULT_WAIT`].
    pub const NEW: Self = Self {
        state: 0,
        timer: 0.0,
        delay: DEFAULT_WAIT,
    };

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

/// What a walking unit's update needs: its layers below the action layer (its place and, if it
/// moves, its movement state), who it is, and the world; the planners and the world's time for
/// a path request (None where the caller has none).
#[derive(Debug)]
pub struct Walk<'a> {
    /// The unit's layers: its place ([`Body::position`]) and its mover, if it moves.
    pub body: &'a mut Body,
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
    /// x, y, z on the map: the unit's place as it is now, after any step of its walk.
    #[must_use]
    pub const fn position(&self) -> [f32; 3] {
        self.body.position()
    }

    /// Sets the walking unit's animation speed (`+0x7b8`, the animated layer in its mover).
    pub const fn set_anim_speed(&mut self, speed: f32) {
        if let Some(m) = self.body.mover_mut() {
            m.animated.speed = speed;
        }
    }

    /// The same walk for a call that ends before this one is used again.
    pub fn reborrow(&mut self) -> Walk<'_> {
        Walk {
            body: &mut *self.body,
            me: self.me,
            dt: self.dt,
            world: &mut *self.world,
            plan: self.plan.as_mut().map(|p| p.reborrow()),
        }
    }
}

/// Where a `MoveTo` goes: a target object or a point (`x, y, z` on the map).
#[derive(Clone, Copy, Debug, PartialEq, Diff)]
pub enum Goal {
    /// A target object.
    Target(i32),
    /// A point.
    Point([f32; 3]),
}

/// A starting `MoveTo`'s update (state 3): unless the unit is within `range` of its goal already, it
/// asks for a path (`FUN_14114e410`) and turns to state 4, whose `set_state` runs the update again at
/// once ([`walk_or_give_up`]); returns the `MoveTo`'s state after it, None when the motion step
/// gave up in that update (the `MoveTo` is in state 4, its sprite shown; the owner's answer
/// decides what follows). In range is the footprints' squared gap (`FUN_14118f8c0`) to a target
/// object, the gap from the unit's footprint to a point, at most `range` squared. A unit already in
/// range ends the `MoveTo` (`set_state` 1, code 0) when it has a unit AI not on order 711 or 733
/// ([`start_walk_asked`]; here it does); a `MoveTo` that is not `quiet` (its `+0x60` clear) then
/// shows the unit's idle sprite (slot 0x68 with slot 0x798's: the master's standing sprite,
/// [`Mover::shows_standing`]; the sprite itself and `+0x7b8`, cleared, are the caller's).
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
    start_walk_shown(walk, (goal, range, quiet), ask, None)
}

/// [`start_walk_asked`] whose turn to state 4 shows the `MoveTo`'s sprite (`shown`, `+0x58`):
/// its `set_state(4)` (`0x140aa29e0`) shows it with slot 0x68 ([`Shown::show`]) before it runs
/// the update again, so the motion step reads the top speed with that sprite. A unit already in
/// range turns to state 1 and shows none of it.
///
/// # Errors
///
/// A target that is gone, a unit that does not move, and what the motion step does not model.
pub fn start_walk_shown(
    walk: Walk<'_>,
    (goal, range, quiet): (Goal, f32, bool),
    ask: PathAsk,
    shown: Option<Shown<'_>>,
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
    let now = walk.plan.as_ref().map_or(0, |p| p.now);
    let mover = walk.body.mover_mut().ok_or(crate::world::WALKER_UNMOVING)?;
    let here = mover.position();
    let ground = [mover.stats.radius[0], mover.stats.radius[1]];
    let squared = if target == -1 {
        let gap = footprint_gap(here, ground, point, radius);
        gap * gap
    } else {
        footprint_gap_squared(here, ground, point, radius)
    };
    if ask.ends_in_range && squared <= range * range {
        if !quiet {
            mover.shows_standing();
        }
        return Ok(Some(MoveTo::Arrived));
    }
    mover.request_path_asked(here, point, range, target, now, ask);
    // the MoveTo's point (`+0x38`): the target's place when it asks for the path
    mover.move_to_point = (target != -1).then_some(point);
    if let Some(mut shown) = shown {
        shown.show(walk.body.move_stats_mut());
    }
    walk_or_give_up(walk)
}

/// A `MoveTo`'s sprite outside the sprites' table.
const SPRITE_LACKING: GenieError =
    GenieError::missing_data("a MoveTo's sprite outside the sprites' table");

/// The unit's sprite, the Sprite stream and the sprites' table: an action that holds its
/// `MoveTo`'s sprite as an id shows it through them ([`Sprites::shown`]).
#[derive(Debug)]
pub struct Sprites<'a> {
    /// The unit's sprite.
    pub sprite: &'a mut Option<ActiveSprite>,
    /// The Sprite stream.
    pub rng: &'a mut Well512,
    /// The sprites' table.
    pub graphics: &'a std::collections::BTreeMap<i16, Graphic>,
}

impl Sprites<'_> {
    /// The view that shows the sprite `id` (None for none) at a `MoveTo`'s `set_state(4)`.
    ///
    /// # Errors
    ///
    /// A sprite the table lacks.
    pub fn shown(&mut self, id: Option<i16>) -> Result<Shown<'_>, GenieError> {
        let graphic = match id {
            Some(id) => Some(
                self.graphics
                    .get(&id)
                    .copied()
                    .ok_or_else(|| SPRITE_LACKING.with("sprite", id))?,
            ),
            None => None,
        };
        Ok(Shown {
            sprite: &mut *self.sprite,
            rng: &mut *self.rng,
            graphic,
        })
    }
}

/// The unit's sprite, the Sprite stream and the sprite a walk shows: a `MoveTo` that turns to
/// state 4 shows its sprite (`0x140aa29e0`, slot 0x68 with `+0x58`).
#[derive(Debug)]
pub struct Shown<'a> {
    /// The unit's sprite.
    pub sprite: &'a mut Option<ActiveSprite>,
    /// The Sprite stream.
    pub rng: &'a mut Well512,
    /// The `MoveTo`'s sprite.
    pub graphic: Option<Graphic>,
}

impl Shown<'_> {
    /// The `MoveTo` turned to state 4: the unit shows its sprite ([`new_sprite`]) and reads its
    /// top speed (`stats`) with it. A `MoveTo` without one (`+0x58` null) is not modelled: slot
    /// 0x68 would show the master's standing sprite (`+0x470`); genie shows nothing.
    pub fn show(&mut self, stats: Option<&mut MoveStats>) {
        if let Some(g) = self.graphic {
            new_sprite(self.sprite, stats, g, self.rng);
        }
    }
}

/// A moving `MoveTo`'s update (state 4, `0x140aa2a70` from `LAB_140aa320c`): the motion step
/// (slot 193); returns the `MoveTo`'s state after it, None when the motion step gave up (it
/// returns 1; the owner decides: `set_state` 2 or 13). A unit that arrived ends the `MoveTo`
/// (`set_state` 1, `0x140aa29e0`, which also sets the unit's idle sprite and clears its `+0x7b8`,
/// and `FUN_14114d810` its facet; the slice holds neither). A `MoveTo` without its path is not
/// modelled.
pub fn walk_or_give_up(walk: Walk<'_>) -> Result<Option<MoveTo>, GenieError> {
    let Walk {
        body,
        me,
        dt,
        world,
        mut plan,
    } = walk;
    let mover = body.mover_mut().ok_or(crate::world::WALKER_UNMOVING)?;
    if mover.paths.is_empty() {
        return Err(GenieError::unsupported("the path request (FUN_14114e410)"));
    }
    Ok(match mover.motion(&me, dt, world, plan.as_mut())? {
        Motion::Moving => Some(MoveTo::Moving),
        Motion::Arrived => Some(MoveTo::Arrived),
        Motion::Done => None,
    })
}

/// A `MoveTo` sub-action's update (`0x140aa2a70`, which an action's sub-action list runs,
/// `FUN_140a992b0`) and the code the action reads: a start asks for its path toward `goal`
/// within `range` (`quiet` its `+0x60`; a `MoveTo` that waited once, `+0x6c` 1, counts as having
/// asked again, 2), shows its sprite (`shown`, [`start_walk_shown`]) and walks; a move runs the
/// motion step
/// ([`walk_or_give_up`]). The motion step giving up makes it wait or fail
/// ([`MoveWait::give_up`] with `waits` and `stance`). A wait adds the walk's dt onto its timer
/// and at its delay asks again (`set_state(3)`; `COMISS`, `JC`: a NaN timer waits on). The code:
/// 0 while it walks or waits, 1 once it ended or without one, 3 once it failed.
///
/// # Errors
///
/// What the path request and the motion step do not model.
pub fn sub_move_to_update(
    (move_to, move_wait): (&mut Option<MoveTo>, &mut MoveWait),
    walk: &mut Walk<'_>,
    goal: Goal,
    (range, quiet): (f32, bool),
    (waits, stance): (bool, u8),
    shown: Option<Shown<'_>>,
) -> Result<u8, GenieError> {
    Ok(match *move_to {
        Some(MoveTo::Start) => {
            if move_wait.state == 1 {
                move_wait.state = 2;
            }
            *move_to = Some(MoveTo::Moving);
            let ask = PathAsk::default();
            let started = start_walk_shown(walk.reborrow(), (goal, range, quiet), ask, shown)?;
            *move_to = Some(started.unwrap_or_else(|| move_wait.give_up(waits, stance)));
            0
        }
        Some(MoveTo::Moving) => {
            let walked = walk_or_give_up(walk.reborrow())?;
            *move_to = Some(walked.unwrap_or_else(|| move_wait.give_up(waits, stance)));
            0
        }
        Some(MoveTo::Waiting) => {
            move_wait.timer += walk.dt;
            if move_wait.delay <= move_wait.timer {
                move_wait.timer = 0.0;
                *move_to = Some(MoveTo::Start);
            }
            0
        }
        Some(MoveTo::Failed) => 3,
        Some(MoveTo::Arrived) | None => 1,
    })
}

/// A target leaves the `MoveTo`'s point when it is more than this off it on either axis
/// (`DAT_142eb6e58`, `DAT_142e7f384`).
const OFF_LOW: f32 = crate::math::image::move_to::OFF_LOW;
const OFF_HIGH: f32 = crate::math::image::move_to::OFF_HIGH;

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
    let mover = walk.body.mover_mut().ok_or(crate::world::WALKER_UNMOVING)?;
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
/// obstruction manager's buffer and ignore list as the last `FUN_1410562d0` and `FUN_1410562e0`
/// left them, [`Spatial::manager_grow`], [`Spatial::manager_ignore`]), and walks on;
/// else it tells the unit (notify 0x1fc), turns to state 3 and the motion step runs on. True when
/// the `MoveTo` turns to state 3.
pub fn target_moved(walk: &Walk<'_>, target: i32, quiet: bool) -> Result<Follow, GenieError> {
    let t = walk
        .world
        .get_known(target)?
        .ok_or(GenieError::unsupported("a MoveTo whose target is gone"))?;
    let point = walk
        .body
        .mover()
        .ok_or(crate::world::WALKER_UNMOVING)?
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
    let mover = walk.body.mover().ok_or(crate::world::WALKER_UNMOVING)?;
    if mover.walks_to_path_end()? {
        let me = walk.position();
        let (a, b) = ([me[0], me[1]], [t.position[0], t.position[1]]);
        let grow = walk.world.manager_grow();
        walk.world.check_known_records([
            a[0].min(b[0]) - grow,
            a[1].min(b[1]) - grow,
            a[0].max(b[0]) + grow,
            a[1].max(b[1]) + grow,
        ])?;
        let ignore = walk.world.manager_ignore();
        if !walk
            .world
            .segment_blocked(a, b, grow, mover.stats.restriction_mask, &ignore)
        {
            return Ok(Follow::PathEnd(t.position));
        }
    }
    Ok(Follow::Again)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::spatial::Placed;

    /// A `MoveTo` sub-action ([`sub_move_to_update`]) on house 10028 within 0.1 (the walker 0.8
    /// off it) with [`WalkRig`]'s walker: a start asks for its path (a `MoveTo` that waited
    /// once, `+0x6c` 1, counts as having asked again, 2) and moves; a move runs the motion step
    /// and keeps moving; both give code 0. A wait runs by the walk's dt and asks again at its
    /// delay. A `MoveTo` that ended, or none, gives 1, a failed one 3.
    ///
    /// [`WalkRig`]: crate::movement::tests::WalkRig
    #[test]
    fn a_sub_move_to_starts_moves_waits_and_reports_its_code() {
        use crate::movement::tests::WalkRig;
        let mut rig = WalkRig::on_the_path();
        let mut move_to = Some(MoveTo::Start);
        let mut wait = MoveWait {
            state: 1,
            timer: 0.0,
            delay: 1.0,
        };
        let goal = Goal::Target(10028);
        let mut update = |m: &mut Option<MoveTo>, w: &mut MoveWait| {
            sub_move_to_update((m, w), &mut rig.walk(4), goal, (0.1, true), (true, 0), None)
        };
        assert_eq!(update(&mut move_to, &mut wait), Ok(0));
        assert_eq!((move_to, wait.state), (Some(MoveTo::Moving), 2));
        assert_eq!(update(&mut move_to, &mut wait), Ok(0));
        assert_eq!(move_to, Some(MoveTo::Moving));
        // the walk is a tenth of a second: the wait of 0.15 asks again on its second update
        move_to = Some(MoveTo::Waiting);
        wait = MoveWait {
            state: 1,
            timer: 0.0,
            delay: 0.15,
        };
        assert_eq!(update(&mut move_to, &mut wait), Ok(0));
        assert_eq!((move_to, wait.timer), (Some(MoveTo::Waiting), 0.1));
        assert_eq!(update(&mut move_to, &mut wait), Ok(0));
        assert_eq!((move_to, wait.timer), (Some(MoveTo::Start), 0.0));
        wait.timer = f32::NAN;
        move_to = Some(MoveTo::Waiting);
        assert_eq!(update(&mut move_to, &mut wait), Ok(0));
        assert_eq!(move_to, Some(MoveTo::Waiting));
        for (m, code) in [
            (None, 1),
            (Some(MoveTo::Arrived), 1),
            (Some(MoveTo::Failed), 3),
        ] {
            move_to = m;
            assert_eq!(update(&mut move_to, &mut wait), Ok(code));
        }
    }

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

    pub(super) fn sheep(x: f32, y: f32) -> Placed {
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

    /// `0x140aa2a70`'s check of its target in state 4: a target within half a tile of the
    /// `MoveTo`'s point stays; off it, a quiet `MoveTo` asks for a path again, and one of its own
    /// moves its path's end to the target while the line to it is clear, else asks again. A
    /// target gone and a point not held are not modelled.
    #[test]
    fn a_target_off_the_move_to_s_point_asks_again_or_moves_the_path_s_end() {
        let open = vec![4; 120 * 120];
        let mut walled = open.clone();
        for y in 85..92 {
            walled[y * 120 + 29] = 1;
        }
        let moved = |classes: &[u8], point, target, quiet| {
            let mut world = crate::movement::tests::tiled_world(classes.to_vec());
            let mut mover = crate::movement::tests::builder_9177();
            mover.move_to_point = point;
            let mut mover = Body::new([28.0, 89.0, 0.0], None, Some(mover));
            let walk = Walk {
                body: &mut mover,
                me: Me {
                    id: 9177,
                    group: -1,
                    action_state: 4,
                },
                dt: 0.1,
                world: &mut world,
                plan: None,
            };
            target_moved(&walk, target, quiet)
        };
        let on = Some([30.4, 88.0, 0.0]);
        let off = Some([30.6, 88.0, 0.0]);
        assert_eq!(moved(&open, on, 10028, true), Ok(Follow::Stays));
        assert_eq!(moved(&open, on, 10028, false), Ok(Follow::Stays));
        assert_eq!(moved(&open, off, 10028, true), Ok(Follow::Again));
        assert_eq!(
            moved(&open, off, 10028, false),
            Ok(Follow::PathEnd([30.0, 88.0, 0.0]))
        );
        assert_eq!(moved(&walled, off, 10028, false), Ok(Follow::Again));
        assert!(moved(&open, None, 10028, true).is_err());
        assert!(moved(&open, on, 4242, true).is_err());
    }
}
