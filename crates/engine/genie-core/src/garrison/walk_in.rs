//! The Enter action ([`EnterAction`], `RGE_Action_Enter`): its walk to its target, its wait
//! and its update.

use crate::GenieError;
use crate::garrison::{ENTER_NEAR, HALF_TILE, UNREACHED};
use crate::movement::move_to::{Goal, MoveTo, Walk, start_walk_or_give_up, walk_or_give_up};

/// The Enter action's type (`RGE_Action_Enter`, vtable `0x142ed4db8`), which word 9 adds.
pub const ENTER: i16 = 3;

/// The gap below which the Enter's `MoveTo` starts arrived (`0x140aa2100`, `DAT_142eb99cc`).
pub(super) const ARRIVED: f32 = crate::math::image::move_to::ARRIVED;

/// The Enter action's states the slice models: 1 done (inside), 2 waiting to try again, 4
/// walking to the target.
pub const ENTERED: u8 = 1;
/// The failed Enter (`set_state(13)`), whose update returns 3.
pub const FAILED: u8 = 13;
pub(super) const WAIT: u8 = 2;
pub(super) const WALK: u8 = 4;
/// `set_state(2)`'s wait before the Enter walks again, in seconds (`FUN_140a87320`, the constant
/// `0x3f800000` it stores).
pub(super) const WAIT_SECONDS: f32 = 1.0;
/// The class of siege weapons, whose own place an Enter's point keeps (`FUN_140a85f30`, the
/// short `0xd`).
pub(crate) const SIEGE_CLASS: i16 = 13;

/// An unload of a siege building or tower (class 13: its spot search `FUN_141192d10` and the
/// unit's leave), not modelled.
pub(crate) const SIEGE_UNLOAD: GenieError =
    GenieError::unsupported("an unload of a siege building (class 13)");

/// A unit's Enter action (`RGE_Action_Enter`, type 3): it walks to a building or a transport and
/// enters it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EnterAction {
    /// The building or transport (`+0x20`, its id `+0x30`).
    pub target: i32,
    /// The action state (`+0x18`).
    pub state: u8,
    /// Its `MoveTo` sub-action's state.
    pub move_to: Option<MoveTo>,
    /// `+0x38`..`+0x40`: where it meets the target ([`meeting`]: a building's place; for a
    /// transport where the two meet, its height the target's at the constructor), which a
    /// target that moves leaves.
    ///
    /// [`meeting`]: crate::garrison::meeting::meeting
    pub point: [f32; 3],
    /// The task's work range (`+0x30`): a building's `MoveTo` stops this near, and the enter
    /// needs it.
    pub range: f32,
    /// `+0x44`: state 2's wait, in seconds.
    pub timer: f32,
    /// `+0x68`, 1 at first: a `MoveTo` that fails (code 3) asks once more.
    pub retry: bool,
    /// `+0x6c`: the task's class (its `+0x10`).
    pub task_class: i16,
    /// Its `MoveTo`'s wait after the motion step gave up (`+0x6c`..`+0x74`).
    pub move_wait: crate::movement::move_to::MoveWait,
}

/// What an Enter's update reads of its target.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EnterTarget {
    /// Its state (`+0x640`).
    pub state: u8,
    /// The unit's player counts its player as allied (`FUN_141663dc0`), or it is Gaia's.
    pub allied: bool,
    /// It has room for the unit (slot 0x198).
    pub room: bool,
    /// Its place.
    pub position: [f32; 3],
    /// The unit is in reach of it ([`EnterAction::in_reach`]).
    pub in_reach: bool,
    /// It is a building (type 80); else a transport.
    pub building: bool,
}

/// What an Enter's update leaves to its caller.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EnterStep {
    /// The update returned 1 (state 1): the action list removes it.
    pub done: bool,
    /// The unit reached the target: the caller enters it (`FUN_140a867b0`, whose end is the
    /// action's `set_state(1)`), then the caller's own `set_state(1)`.
    pub enter: bool,
    /// The enter came from the `set_state(4)` of a target that moved, in state 4: the update then
    /// finds no `MoveTo` (code 1) with the unit in reach and enters again (`FUN_140a867b0` on a
    /// unit inside it already: notify 0x1fa and `set_state(1)`), then its own `set_state(1)`.
    pub again: bool,
    /// `set_state(2)`: the unit shows its master's standing sprite (slot 0x68 with `+0x470`).
    pub waits: bool,
    /// `set_state(4)` found the transport where it meets the unit (its [`meeting`] code 1): the
    /// caller asks the transport to move there (slot 0x3d8 within 1, then slot 0x340), the
    /// point's height the transport's.
    ///
    /// [`meeting`]: crate::garrison::meeting::meeting
    pub order: Option<[f32; 3]>,
}

/// What [`EnterAction::walk_again`] (`set_state(4)`) leaves to its caller.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Walking {
    /// The unit is in reach: the caller enters (`FUN_140a867b0`), then `set_state(1)`.
    Enter,
    /// A `MoveTo` to the target started; with a point, the transport is to move there first
    /// ([`EnterStep::order`]).
    Walk(Option<[f32; 3]>),
}

impl EnterAction {
    /// The factory's constructor (`FUN_140a855d0`): the target (an annex's main building, the
    /// caller's, as its add, slot 0x178), the point [`meeting`] gives at the target's height,
    /// the task's range and class, `+0x68` 1; the start (slot 5, `FUN_140a86f20`) and its
    /// `set_state(4)` are [`EnterAction::walk_again`].
    ///
    /// [`meeting`]: crate::garrison::meeting::meeting
    #[must_use]
    pub const fn new(target: i32, point: [f32; 3], range: f32, task_class: i16) -> Self {
        Self {
            target,
            state: WALK,
            move_to: None,
            point,
            range,
            timer: 0.0,
            retry: true,
            task_class,
            move_wait: crate::movement::move_to::MoveWait {
                state: 0,
                timer: 0.0,
                delay: 0.0,
            },
        }
    }

    /// `set_state(4)` (`FUN_140a87320` case 4) after the start's or the update's tests: its
    /// sub-actions go (`FUN_140a98620`; a `MoveTo`'s teardown stops the unit, the caller's), and
    /// its room test repeats theirs, which nothing between changes. A unit in reach of the
    /// target (`FUN_140a85c30`) enters it ([`Walking::Enter`]). Else a building's `MoveTo` walks
    /// to it within the task's range (`FUN_140aa0df0`); a transport's takes the point and code
    /// of [`meeting`] (`FUN_140a85f30`, the action's `+0x38` and `+0x3c`) and walks to the
    /// transport within 0 (`FUN_140aa0d30` with range 0, then slot 0x68 with the transport: its
    /// state 3 goes to the transport's place; a point below 0 would make `FUN_140aa0df0` with the
    /// unit's slot 0x268, 0, alike), and code 1 asks the transport to move to the point. Both
    /// `MoveTo`s are quiet (`FUN_140a7a170` with 1) and leave a target that moves (`+0x61` 2);
    /// their start ([`EnterAction::walk_start`]) is `0x140aa2100`'s, by the footprints' gap
    /// `gap` and the unit's speed.
    ///
    /// [`meeting`]: crate::garrison::meeting::meeting
    pub fn walk_again(
        &mut self,
        t: &EnterTarget,
        gap: f32,
        speed: f32,
        meeting: &mut impl FnMut() -> Result<([f32; 2], u8), GenieError>,
    ) -> Result<Walking, GenieError> {
        self.move_to = None;
        self.state = WALK;
        if t.in_reach {
            return Ok(Walking::Enter);
        }
        let code = if t.building {
            0
        } else {
            let (point, code) = meeting()?;
            self.point[..2].copy_from_slice(&point);
            code
        };
        self.move_to = Some(Self::walk_start(gap, speed)?);
        Ok(Walking::Walk((code == 1).then_some([
            self.point[0],
            self.point[1],
            t.position[2],
        ])))
    }

    /// The `MoveTo`'s start (`0x140aa2100` with 1): beyond [`ARRIVED`] of the target's footprint
    /// (`gap`) state 3, or 13 for a unit without speed (not modelled); else state 1.
    pub(super) fn walk_start(gap: f32, speed: f32) -> Result<MoveTo, GenieError> {
        if ARRIVED < gap {
            if speed == 0.0 {
                return Err(GenieError::unsupported(
                    "an Enter's MoveTo of a unit without speed",
                ));
            }
            return Ok(MoveTo::Start);
        }
        Ok(MoveTo::Arrived)
    }

    /// The range its `MoveTo` stops within: the task's for a building, 0 for a transport.
    pub(super) const fn walk_range(&self, building: bool) -> f32 {
        if building { self.range } else { 0.0 }
    }

    /// `set_state(2)`: the wait of [`WAIT_SECONDS`] (the standing sprite is the caller's,
    /// [`EnterStep::waits`]).
    pub(super) const fn wait(&mut self, step: &mut EnterStep) {
        self.move_to = None;
        self.state = WAIT;
        self.timer = WAIT_SECONDS;
        step.waits = true;
    }

    /// `FUN_140a85c30`: the unit is in reach of the target: their footprints' squared gap
    /// (`FUN_14118f8c0`; `COMISS`/`JBE`: a NaN gap is not out) at most the range squared; for a
    /// range of at most 1 (`COMISS`/`JC`: a NaN range is in reach) to a target with speed (slot
    /// 0x208, above 0) the tiles about the two ([`walled_off`], `tiles`) must not hold exactly
    /// two buildings.
    ///
    /// [`walled_off`]: crate::garrison::meeting::walled_off
    pub fn in_reach(
        range: f32,
        gap_squared: f32,
        target_speed: f32,
        tiles: impl FnOnce() -> Result<bool, GenieError>,
    ) -> Result<bool, GenieError> {
        if range * range < gap_squared {
            return Ok(false);
        }
        if range <= ENTER_NEAR && 0.0 < target_speed {
            return Ok(!tiles()?);
        }
        Ok(true)
    }

    /// One update (slot 6, `FUN_140a87a80`). It first checks its target
    /// ([`EnterAction::held_target`]). State 1 is done; state 2 waits its timer (by the world's
    /// `dt`), then walks again ([`EnterAction::walk_again`]). State 4: a target off the action's
    /// point ([`EnterAction::target_moved`]) walks it again; then the `MoveTo` updates
    /// ([`EnterAction::walk_code`]): codes 1 and 2 enter in reach ([`EnterStep::enter`]), else
    /// wait (`set_state(2)`); code 3 (a `MoveTo` that failed) walks again once (`+0x68`), then
    /// waits; codes 4 and 5 let the target go (state 11), which is not modelled. State 13
    /// returns 3, which is not modelled. `gap` and `speed` start a new `MoveTo`
    /// ([`EnterAction::walk_start`]); `meeting` gives a transport's point ([`meeting`]) and
    /// `reaches` its reach test there ([`EnterAction::again`]).
    ///
    /// [`meeting`]: crate::garrison::meeting::meeting
    pub fn update(
        &mut self,
        mut walk: Walk<'_>,
        target: Option<EnterTarget>,
        (gap, speed): (f32, f32),
        mut meeting: impl FnMut() -> Result<([f32; 2], u8), GenieError>,
        mut reaches: impl FnMut(&mut Walk<'_>, [f32; 3]) -> Result<bool, GenieError>,
        (waits, stance): (bool, u8),
    ) -> Result<EnterStep, GenieError> {
        let mut step = EnterStep::default();
        let t = Self::held_target(target)?;
        match self.state {
            // state 1 returns 1, state 13 returns 3: either way the list removes the action
            ENTERED | FAILED => step.done = true,
            WAIT => {
                self.timer -= walk.dt;
                if self.timer <= 0.0 {
                    self.again(
                        &mut walk,
                        &t,
                        (gap, speed),
                        &mut meeting,
                        &mut reaches,
                        &mut step,
                    )?;
                }
            }
            WALK => {
                let walked_again = self.target_moved(&t);
                if walked_again {
                    self.again(
                        &mut walk,
                        &t,
                        (gap, speed),
                        &mut meeting,
                        &mut reaches,
                        &mut step,
                    )?;
                }
                let range = self.walk_range(t.building);
                match self.walk_code(&mut walk, range, (waits, stance))? {
                    // the walk again's enter put the unit inside, at the target's place: still
                    // in reach, it enters again
                    1 | 2 if walked_again && step.enter => step.again = true,
                    1 | 2 if t.in_reach => step.enter = true,
                    // a failed MoveTo is none a walk again made in this update
                    3 if self.retry => {
                        self.retry = false;
                        self.again(
                            &mut walk,
                            &t,
                            (gap, speed),
                            &mut meeting,
                            &mut reaches,
                            &mut step,
                        )?;
                    }
                    1..=3 => self.wait(&mut step),
                    _ => {}
                }
            }
            _ => return Err(GenieError::unsupported("an Enter state not modelled")),
        }
        Ok(step)
    }

    /// The update's test of its target first (`FUN_140a87a80`): one gone, dead (state 3 or
    /// more) or of an unallied player, or without room for the unit, makes the action let it go
    /// and turn to state 11, which is not modelled.
    pub(super) const fn held_target(
        target: Option<EnterTarget>,
    ) -> Result<EnterTarget, GenieError> {
        let Some(t) = target else {
            return Err(GenieError::unsupported(
                "an Enter whose building is gone (state 11)",
            ));
        };
        if 3 <= t.state || !t.allied {
            return Err(GenieError::unsupported(
                "an Enter whose building died or is not allied (state 11)",
            ));
        }
        if !t.room {
            return Err(GenieError::unsupported(
                "an Enter whose building has no room (state 11)",
            ));
        }
        Ok(t)
    }

    /// State 4's test of its target: off the action's point (`FUN_140a7bbf0`: any of x, y and
    /// height not equal) by more than [`HALF_TILE`] on an axis (`FUN_140a7bd10`: `COMISS`/`JA`,
    /// a NaN is not).
    pub(super) fn target_moved(&self, t: &EnterTarget) -> bool {
        let [x, y, z] = t.position;
        let moved = |d: f32| HALF_TILE < d.abs();
        t.position != self.point
            && (moved(x - self.point[0]) || moved(y - self.point[1]) || moved(z - self.point[2]))
    }

    /// State 4's `MoveTo` update (`FUN_140a992b0`) within `range`: its code, 1 once it ended or
    /// without one, 3 once it failed, else 0. A start asks for the path to the target, a move
    /// runs the motion step; a motion step that gives up makes it wait or fail
    /// ([`crate::movement::move_to::MoveWait::give_up`] with `waits` and `stance`). A wait adds
    /// the world's dt onto its timer and at its delay asks again (`set_state(3)`; `COMISS`, `JC`:
    /// a NaN timer waits on).
    pub(super) fn walk_code(
        &mut self,
        walk: &mut Walk<'_>,
        range: f32,
        (waits, stance): (bool, u8),
    ) -> Result<u8, GenieError> {
        Ok(match self.move_to {
            None | Some(MoveTo::Arrived) => 1,
            Some(MoveTo::Start) => {
                if self.move_wait.state == 1 {
                    self.move_wait.state = 2;
                }
                self.move_to = Some(MoveTo::Moving);
                let goal = Goal::Target(self.target);
                self.move_to = Some(
                    match start_walk_or_give_up(walk.reborrow(), goal, range, true)? {
                        Some(state) => state,
                        None => self.move_wait.give_up(waits, stance),
                    },
                );
                0
            }
            Some(MoveTo::Moving) => {
                self.move_to = Some(match walk_or_give_up(walk.reborrow())? {
                    Some(state) => state,
                    None => self.move_wait.give_up(waits, stance),
                });
                0
            }
            Some(MoveTo::Failed) => 3,
            Some(MoveTo::Waiting) => {
                self.move_wait.timer += walk.dt;
                if self.move_wait.delay <= self.move_wait.timer {
                    self.move_wait.timer = 0.0;
                    self.move_to = Some(MoveTo::Start);
                }
                0
            }
        })
    }

    /// The update's `set_state(4)` ([`EnterAction::walk_again`]): its `MoveTo`'s teardown stops
    /// the unit (slot 0x5d0); an enter is the caller's. A transport's order first tests that the
    /// transport reaches within 1 of the point's tile (slot 0x3d8, `FUN_14114ffc0` with the
    /// point truncated: `reaches`); the move itself (slot 0x340) is the caller's
    /// ([`EnterStep::order`]). A transport that cannot reach it makes `set_state(13)`, which is
    /// not modelled.
    pub(super) fn again(
        &mut self,
        walk: &mut Walk<'_>,
        t: &EnterTarget,
        (gap, speed): (f32, f32),
        meeting: &mut impl FnMut() -> Result<([f32; 2], u8), GenieError>,
        reaches: &mut impl FnMut(&mut Walk<'_>, [f32; 3]) -> Result<bool, GenieError>,
        step: &mut EnterStep,
    ) -> Result<(), GenieError> {
        if self.move_to.is_some()
            && let Some(m) = walk.mover.as_deref_mut()
        {
            m.stop()?;
        }
        match self.walk_again(t, gap, speed, meeting)? {
            Walking::Enter => step.enter = true,
            Walking::Walk(Some(at)) => {
                if !reaches(walk, at)? {
                    return Err(UNREACHED);
                }
                step.order = Some(at);
            }
            Walking::Walk(None) => {}
        }
        Ok(())
    }
}
