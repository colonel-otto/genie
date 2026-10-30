//! The `MoveTo` as an action's sub-action: its constructor (`FUN_140aa0d30`, made quiet) and its
//! update through the owner's sub-action list (`FUN_140a992b0`).

use crate::GenieError;
use crate::movement::PathAsk;
use crate::movement::move_to::action::ARRIVED;
use crate::movement::move_to::{
    Follow, MoveTo, MoveToAction, MoveWait, Shown, TargetMoved, Walk, ask_walk, walk_or_give_up,
};
use crate::objects::sprite::Graphic;
use crate::unit_ai::{ACTION_COMPLETED, ACTION_FAILED, ActionKind, Inbox, TARGET_MOVED};

/// A sub-action `MoveTo` that is not quiet letting its target go: its release (slot 0x180) would
/// be its owner's, and no owner makes one.
const SUB_RELEASE: GenieError =
    GenieError::unreachable("a sub-action MoveTo that is not quiet letting its target go");

/// The gap `FUN_14118f680` gives a `MoveTo` without a target, one to a point: the largest
/// float, which no start counts as arrived ([`MoveToAction::added`]).
pub const NO_TARGET_GAP: f32 = crate::math::image::garrison::FAR;

/// What the motion step's giving up reads of the unit ([`MoveWait::give_up`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GiveUp {
    /// A human player's villager (`LAB_140aa320c`: `FUN_141659cf0` 0, the master's class 4),
    /// which waits once before it asks again.
    pub waits: bool,
    /// The unit AI's stance (`+0x1a9`), 0 without one: below 2 it waits a second, else two.
    pub stance: u8,
}

impl MoveToAction {
    /// A `MoveTo` to `point` within `range` that an action makes its sub-action
    /// (`FUN_140aa0d30`, then `FUN_140a7a170` with 1: quiet, `+0x60`), with `sprite` (`+0x58`) as
    /// the constructor resolves it; the action list starts it in state 3 (`FUN_140a977e0`).
    #[must_use]
    pub const fn sub_to_point(point: [f32; 3], range: f32, sprite: Option<Graphic>) -> Self {
        Self {
            state: MoveTo::Start,
            point,
            target: -1,
            range,
            sprite,
            quiet: true,
            on_target_moved: TargetMoved::Follows,
            wait: MoveWait::NEW,
        }
    }

    /// Its start as its owner adds it to its sub-actions (`FUN_140a977e0`): a unit in a state
    /// (`+0x640`) of 3 or later adds none (the game deletes the new `MoveTo`). Else its start
    /// (slot 5 with 1, `0x140aa2100`) sets its state by `gap`, the unit's from the target's
    /// footprint (`FUN_14118f680`; [`NO_TARGET_GAP`] for a `MoveTo` to a point): within
    /// [`ARRIVED`] it arrived (1; `COMISS`, `JC`: a NaN gap is far), else it starts (3) with
    /// `speed` (slot 0x208) and fails (13) without (`UCOMISS`, `JP`: a NaN speed starts). A quiet
    /// `MoveTo` shows no standing sprite in states 1 and 13 (`0x140aa29e0`).
    #[must_use]
    pub const fn added(self, unit_state: u8, gap: f32, speed: f32) -> Option<Self> {
        if 3 <= unit_state {
            return None;
        }
        let state = if gap <= ARRIVED {
            MoveTo::Arrived
        } else if speed == 0.0 {
            MoveTo::Failed
        } else {
            MoveTo::Start
        };
        Some(Self { state, ..self })
    }

    /// Its update as an action's sub-action (`0x140aa2a70`, which the action's sub-action list
    /// runs, `FUN_140a992b0`) and the code the action reads. First the check of its target by
    /// the target's object state (`target_state`, [`MoveToAction::check_held`]): a target let go
    /// ends the update with 5 (a sub-action `MoveTo` is quiet). Then a start asks for its path toward
    /// its target or point within its range by what the unit's AI asks (`ask`; a `MoveTo` that
    /// waited once, `+0x6c` 1, counts as having asked again, 2), shows its sprite (`shown`,
    /// [`ask_walk`]) and walks; a
    /// move checks its target ([`MoveToAction::check_target`]: a `MoveTo` that asks again ends
    /// its update there, one whose path's end could not follow tells the unit at once through
    /// `inbox`, notify 0x1fc, whether quiet or not) and runs the motion step
    /// ([`walk_or_give_up`]), which moving on leaves it in the state the check left. The motion
    /// step giving up makes it wait or fail ([`MoveWait::give_up`] with `gives_up`). A wait adds
    /// the walk's dt onto its timer and at its delay asks again (`set_state(3)`; `COMISS`, `JC`: a
    /// NaN timer waits on). The code: 0 while it walks or waits, 1 once it ended, 3 once it
    /// failed, 5 once its target was let go.
    ///
    /// # Errors
    ///
    /// A target the world lacks, and what the path request and the
    /// motion step do not model.
    pub fn sub_update(
        &mut self,
        walk: &mut Walk<'_>,
        ask: PathAsk,
        gives_up: GiveUp,
        target_state: Option<u8>,
        shown: Option<Shown<'_>>,
        inbox: &mut Inbox<'_>,
    ) -> Result<u8, GenieError> {
        let target = self.target;
        if let Some(code) = self.check_held(target_state, inbox)? {
            // no owner makes a sub-action MoveTo to an object that is not quiet, which would
            // owe its target the release (FUN_140a7a230)
            if !self.quiet {
                return Err(SUB_RELEASE.with("target", target));
            }
            return Ok(code);
        }
        let GiveUp { waits, stance } = gives_up;
        Ok(match self.state {
            MoveTo::Start => {
                if self.wait.state == 1 {
                    self.wait.state = 2;
                }
                let goal = self.walks_to();
                self.state = MoveTo::Moving;
                let (started, asked) = ask_walk(walk, (goal, self.range, self.quiet), ask, shown)?;
                if let Some(point) = asked {
                    self.point = point;
                }
                self.state = started.unwrap_or_else(|| self.wait.give_up(waits, stance));
                0
            }
            MoveTo::Moving => {
                match self.check_target(walk)? {
                    Follow::Again => return Ok(0),
                    Follow::Blocked => inbox.post(TARGET_MOVED, [0, 0, 0])?,
                    Follow::Stays | Follow::PathEnd(_) => {}
                }
                match walk_or_give_up(walk.reborrow())? {
                    Some(MoveTo::Moving) => {}
                    Some(other) => self.state = other,
                    None => self.state = self.wait.give_up(waits, stance),
                }
                0
            }
            MoveTo::Waiting => {
                self.wait.timer += walk.dt;
                if self.wait.delay <= self.wait.timer {
                    self.wait.timer = 0.0;
                    self.state = MoveTo::Start;
                }
                0
            }
            MoveTo::Failed => {
                self.post_end(inbox, ACTION_FAILED)?;
                3
            }
            MoveTo::Arrived => {
                self.post_end(inbox, ACTION_COMPLETED)?;
                1
            }
        })
    }

    /// A `MoveTo` that is not quiet tells its unit's AI how it ended (`0x140aa2a70` in state 1
    /// and 13: notify `kind`, 0x1fa or 0x1f9, with the move action value and 0, 0).
    fn post_end(&self, inbox: &mut Inbox<'_>, kind: i32) -> Result<(), GenieError> {
        if self.quiet {
            return Ok(());
        }
        inbox.post(kind, [ActionKind::Move.value(), 0, 0])
    }

    /// A quiet sub-action `MoveTo` in `state` to (0, 0, 0) without a sprite, for a test that
    /// reads only the state.
    #[cfg(test)]
    pub(crate) const fn sub_in(state: MoveTo) -> Self {
        Self {
            state,
            ..Self::sub_to_point([0.0; 3], 0.0, None)
        }
    }
}
