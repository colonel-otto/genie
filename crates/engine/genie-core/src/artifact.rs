//! The Artifact action (type 107): the default task of livestock, which keeps checking whether a
//! unit of another player is close enough to take the animal over (the search is the caller's,
//! [`crate::spatial::Spatial::artifact_owner`]).
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
    /// (set_state 6). A move to an object (set_state 5) is not modelled. Returns whether a MoveTo
    /// went, whose teardown (`0x140aa1f30`) stops the unit (slot 0x5d0, the caller's).
    pub fn take_point(&mut self, point: [f32; 3]) -> bool {
        self.goal = Some(point);
        let walked = self.move_to.replace(MoveTo::Start).is_some();
        self.state = CHECK;
        walked
    }

    /// Slot 0x38 (`FUN_140aad1a0`), as the unit's stop (`FUN_14106fa60`) calls it before a new
    /// owner takes the animal: its sub-actions go (a MoveTo's teardown would stop the unit, which
    /// is not modelled), with the point and the target, and it checks again (set_state 6). The
    /// unit's standing sprite (slot 0x68) is not held.
    pub fn stop(&mut self) -> Result<(), Unsupported> {
        if self.move_to.is_some() {
            return Err(Unsupported(
                "a captured animal walking its MoveTo (its teardown)",
            ));
        }
        self.goal = None;
        self.state = CHECK;
        Ok(())
    }

    /// One update (`0x140aad250`) with the tick's `dt_ms`. Its MoveTo goes first: it starts its
    /// walk, walks on, and once it ended (its update in state 1 returns 1) the sub-actions go, with
    /// the point and the target (slot 0x68 with none). Then the check (state 6): an animal of the
    /// combat type whose `+0x924` is 0 (`FUN_14107cc90`, `searches` false) never changes hands; any
    /// other asks `owner` (`FUN_140aabc40`) and returns the player that takes it, after its
    /// sub-actions went (`FUN_140a98620`), for the caller's conversion (`FUN_140aab570`). With no
    /// new owner it waits (set_state 10).
    pub fn update(
        &mut self,
        walk: Option<Walk<'_>>,
        dt_ms: u32,
        searches: bool,
        owner: &mut dyn FnMut() -> Result<Option<usize>, Unsupported>,
    ) -> Result<Option<usize>, Unsupported> {
        self.walk(walk)?;
        self.step(dt_ms, searches, owner)
    }

    /// The update's first part: its MoveTo sub-action. True when the MoveTo ended: its update in
    /// state 1 posts its completion (0x1fa with the move action value; the Artifact's MoveTo is
    /// not quiet), which the caller sends to the unit AI.
    pub fn walk(&mut self, walk: Option<Walk<'_>>) -> Result<bool, Unsupported> {
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
                return Ok(true);
            }
            None => {}
        }
        Ok(false)
    }

    /// The update's second part: the state's step.
    pub fn step(
        &mut self,
        dt_ms: u32,
        searches: bool,
        owner: &mut dyn FnMut() -> Result<Option<usize>, Unsupported>,
    ) -> Result<Option<usize>, Unsupported> {
        match self.state {
            CHECK => {
                if searches && let Some(new) = owner()? {
                    if self.move_to.is_some() {
                        return Err(Unsupported(
                            "a captured animal walking its MoveTo (its teardown)",
                        ));
                    }
                    self.goal = None;
                    return Ok(Some(new));
                }
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
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_check_finds_no_owner_and_waits_one_second() {
        let mut a = Artifact::new(false);
        a.update(None, 50, true, &mut || Ok(None)).unwrap();
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
        assert!(!a.take_point([28.5, 93.5, 0.0]));
        assert_eq!(
            (a.state, a.timer, a.move_to, a.goal),
            (6, 0.4, Some(MoveTo::Start), Some([28.5, 93.5, 0.0]))
        );
        // a second move while the first walks: its MoveTo goes, which stops the unit (the
        // sheep 9520 in 768)
        let mut walking = Artifact {
            move_to: Some(MoveTo::Moving),
            ..a
        };
        assert!(walking.take_point([28.5, 94.5, 0.0]));
        assert_eq!(walking.move_to, Some(MoveTo::Start));
        // its walk needs the animal's movement state
        assert!(a.update(None, 50, true, &mut || Ok(None)).is_err());
        // a MoveTo that ended goes with the point, then the check waits again
        a.move_to = Some(MoveTo::Arrived);
        a.update(None, 50, true, &mut || Ok(None)).unwrap();
        assert_eq!((a.state, a.timer, a.move_to, a.goal), (10, 1.0, None, None));
    }

    #[test]
    fn gaia_waits_half_a_second() {
        let mut a = Artifact::new(true);
        a.update(None, 50, true, &mut || Ok(None)).unwrap();
        assert_eq!(a.timer, 0.5);
    }

    #[test]
    fn the_wait_ends_when_the_timer_reaches_zero_then_one_check_follows() {
        let mut a = Artifact::new(false);
        a.update(None, 50, true, &mut || Ok(None)).unwrap();
        // 1.0 - 20 × 0.05 in f32 is -1.56e-7, so the 20th update ends the wait
        let waits = (0..30).take_while(|_| {
            a.update(None, 50, true, &mut || Ok(None)).unwrap();
            a.state == 10
        });
        assert_eq!(waits.count(), 19);
        assert_eq!(a.state, 6);
        a.update(None, 50, true, &mut || Ok(None)).unwrap();
        assert_eq!((a.state, a.timer), (10, 1.0));
    }

    #[test]
    fn a_check_that_finds_a_new_owner_hands_the_animal_over_and_stays_a_check() {
        let mut a = Artifact::new(true);
        let mut asked = 0;
        let found = a
            .update(None, 50, true, &mut || {
                asked += 1;
                Ok(Some(2))
            })
            .unwrap();
        assert_eq!((found, a.state, asked), (Some(2), 6, 1));
        // a combat-type animal whose +0x924 is 0 never asks
        let mut b = Artifact::new(true);
        let found = b.update(None, 50, false, &mut || Ok(Some(2))).unwrap();
        assert_eq!((found, b.state), (None, 10));
        // the conversion's stop: the check again
        b.stop().unwrap();
        assert_eq!(b.state, 6);
    }
}
