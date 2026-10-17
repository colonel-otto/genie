//! The Artifact action (type 107): the default task of livestock, which keeps checking whether a
//! unit of another player is close enough to take the animal over.
//!
//! 158041: `TRIBE_Action_Artifact`, vtable `0x142ed6520`; update `0x140aad250` (slot 6), set_state
//! `0x140aace30` (slot 16). The .dat gives Goat, Goose, Cow and Sheep the default task 0, a task with
//! action type 107. An idle animal holds this action as its top action, so checkpoint word 9 counts
//! `107 + state` for it.

use crate::move_to::{Goal, MoveTo, Walk, start_walk, walk_on};
use crate::unit_ai::Unsupported;

/// One animal's Artifact action, as the checkpoint sees it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Artifact {
    /// 6 checks for a new owner; 10 waits for the timer.
    pub state: u8,
    /// `+0x44`, in seconds.
    pub timer: f32,
    /// Gaia waits half as long between checks.
    pub gaia: bool,
    /// The MoveTo sub-action of a move its player gave the animal, and the action's point
    /// (`+0x38`..`+0x40`) it walks to.
    pub move_to: Option<MoveTo>,
    pub goal: Option<[f32; 3]>,
}

/// The action type the checkpoint adds for an Artifact action.
pub const ARTIFACT_TYPE: i16 = 107;

const CHECK: u8 = 6;
const WAIT: u8 = 10;

impl Artifact {
    /// A new action, about to check for a new owner.
    pub fn new(gaia: bool) -> Self {
        Artifact {
            state: CHECK,
            timer: 0.0,
            gaia,
            move_to: None,
            goal: None,
        }
    }

    /// Slot 8 (`FUN_140aaca30`) on a move to a point, which the action takes: the point, then
    /// set_state(4) (`0x140aace30`): its sub-actions go (`FUN_140a98620`), a MoveTo to the point
    /// within 0 (`FUN_140aa0d30` with its task's sprite, walking; the sprite is not held) is added
    /// and starts (state 3 for an animal away from the point), and the action checks again
    /// (set_state 6). A move to an object (set_state 5) is not modelled.
    pub fn take_point(&mut self, point: [f32; 3]) {
        self.goal = Some(point);
        self.move_to = Some(MoveTo::Start);
        self.state = CHECK;
    }

    /// One update (`0x140aad250`) with the tick's `dt_ms`. Its MoveTo goes first: it starts its
    /// walk, walks on, and once it ended (its update in state 1 returns 1) the sub-actions go, with
    /// the point and the target (slot 0x68 with none). Then the check always finds no new owner:
    /// the slice assumes no unit of another player comes near the animal.
    pub fn update(&mut self, walk: Option<Walk<'_>>, dt_ms: u32) -> Result<(), Unsupported> {
        let need = || Unsupported("a walking animal without its walk");
        match self.move_to {
            Some(MoveTo::Start) => {
                let goal = self
                    .goal
                    .ok_or(Unsupported("an Artifact MoveTo without its point"))?;
                self.move_to = Some(MoveTo::Moving);
                self.move_to = Some(start_walk(
                    walk.ok_or_else(need)?,
                    Goal::Point(goal),
                    0.0,
                    false,
                )?);
            }
            Some(MoveTo::Moving) => self.move_to = Some(walk_on(walk.ok_or_else(need)?)?),
            Some(MoveTo::Arrived) => {
                self.move_to = None;
                self.goal = None;
            }
            None => {}
        }
        match self.state {
            CHECK => {
                self.state = WAIT;
                self.timer = if self.gaia { 0.5 } else { 1.0 };
            }
            WAIT => {
                // world+0x164 is `dt / 1000.0f`; the subtraction is f32
                self.timer -= dt_ms as f32 / 1000.0;
                if self.timer <= 0.0 {
                    self.state = CHECK;
                }
            }
            _ => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_check_finds_no_owner_and_waits_one_second() {
        let mut a = Artifact::new(false);
        a.update(None, 50).unwrap();
        assert_eq!((a.state, a.timer), (10, 1.0));
    }

    #[test]
    fn a_move_s_point_starts_a_moveto_and_the_check_runs_again() {
        // MarvinBlue/426048731's cow 9517 in 1118: MOVE to (28.5, 93.5); slot 8 takes the point
        let mut a = Artifact {
            state: WAIT,
            timer: 0.4,
            ..Artifact::new(false)
        };
        a.take_point([28.5, 93.5, 0.0]);
        assert_eq!(
            (a.state, a.timer, a.move_to, a.goal),
            (6, 0.4, Some(MoveTo::Start), Some([28.5, 93.5, 0.0]))
        );
        // its walk needs the animal's movement state
        assert!(a.update(None, 50).is_err());
        // a MoveTo that ended goes with the point, then the check waits again
        a.move_to = Some(MoveTo::Arrived);
        a.update(None, 50).unwrap();
        assert_eq!((a.state, a.timer, a.move_to, a.goal), (10, 1.0, None, None));
    }

    #[test]
    fn gaia_waits_half_a_second() {
        let mut a = Artifact::new(true);
        a.update(None, 50).unwrap();
        assert_eq!(a.timer, 0.5);
    }

    #[test]
    fn the_wait_ends_when_the_timer_reaches_zero_then_one_check_follows() {
        let mut a = Artifact::new(false);
        a.update(None, 50).unwrap();
        // 1.0 - 20 × 0.05 in f32 is -1.56e-7, so the 20th update ends the wait
        let waits = (0..30).take_while(|_| {
            a.update(None, 50).unwrap();
            a.state == 10
        });
        assert_eq!(waits.count(), 19);
        assert_eq!(a.state, 6);
        a.update(None, 50).unwrap();
        assert_eq!((a.state, a.timer), (10, 1.0));
    }
}
