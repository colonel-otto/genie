//! The `MoveTo` as an action's sub-action: its constructor (`FUN_140aa0d30`, made quiet) and its
//! update through the owner's sub-action list (`FUN_140a992b0`).

use crate::GenieError;
use crate::movement::PathAsk;
use crate::movement::move_to::{
    Follow, MoveTo, MoveToAction, MoveWait, Shown, TargetMoved, Walk, start_walk_shown,
    walk_or_give_up,
};
use crate::objects::sprite::Graphic;
use crate::unit_ai::{Inbox, TARGET_MOVED};

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
            goal: Some(point),
            target: -1,
            range,
            sprite,
            quiet: true,
            on_target_moved: TargetMoved::Follows,
            wait: MoveWait::NEW,
        }
    }

    /// Its update as an action's sub-action (`0x140aa2a70`, which the action's sub-action list
    /// runs, `FUN_140a992b0`) and the code the action reads: a start asks for its path toward
    /// its target or point within its range by what the unit's AI asks (`ask`; a `MoveTo` that
    /// waited once, `+0x6c` 1, counts as having asked again, 2), shows its sprite (`shown`,
    /// [`start_walk_shown`]) and walks; a
    /// move checks its target ([`MoveToAction::check_target`]: a `MoveTo` that asks again ends
    /// its update there, one whose path's end could not follow tells the unit at once through
    /// `inbox`, notify 0x1fc, whether quiet or not) and runs the motion step
    /// ([`walk_or_give_up`]), which moving on leaves it in the state the check left. The motion
    /// step giving up makes it wait or fail ([`MoveWait::give_up`] with `gives_up`). A wait adds
    /// the walk's dt onto its timer and at its delay asks again (`set_state(3)`; `COMISS`, `JC`: a
    /// NaN timer waits on). The code: 0 while it walks or waits, 1 once it ended, 3 once it
    /// failed.
    ///
    /// # Errors
    ///
    /// A `MoveTo` without a goal, a target the world lacks, and what the path request and the
    /// motion step do not model.
    pub fn sub_update(
        &mut self,
        walk: &mut Walk<'_>,
        ask: PathAsk,
        gives_up: GiveUp,
        shown: Option<Shown<'_>>,
        inbox: &mut Inbox<'_>,
    ) -> Result<u8, GenieError> {
        let GiveUp { waits, stance } = gives_up;
        Ok(match self.state {
            MoveTo::Start => {
                if self.wait.state == 1 {
                    self.wait.state = 2;
                }
                let goal = self.walks_to()?;
                self.state = MoveTo::Moving;
                let started =
                    start_walk_shown(walk.reborrow(), (goal, self.range, self.quiet), ask, shown)?;
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
            MoveTo::Failed => 3,
            MoveTo::Arrived => 1,
        })
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
