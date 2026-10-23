//! The Artifact action (type 107): the default task of livestock, which keeps checking whether a
//! unit of another player is close enough to take the animal over (the search is the caller's,
//! [`crate::spatial::Spatial::artifact_owner`]).
//!
//! 158041: `TRIBE_Action_Artifact`, vtable `0x142ed6520`; update `0x140aad250` (slot 6), `set_state`
//! `0x140aace30` (slot 16). The .dat gives Goat, Goose, Cow and Sheep the default task 0, a task with
//! action type 107. An idle animal holds this action as its top action, so checkpoint word 9 counts
//! `107 + state` for it.

use crate::move_to::{Goal, MoveTo, Walk, start_walk, walk_or_give_up};
use crate::unit_ai::Unsupported;

/// What the Artifact's slot 0x68 (the base's `FUN_140a7a230`) did to its targets, for the
/// caller: the action is not quiet (`+0x60` 0, its constructors' `FUN_140a73e90` with 1), so the
/// old target, when the world holds it, lets the unit go (its slot 0x180, `Turns::release`) and
/// a new one in a state below 7 takes it (its slot 0x178, `Turns::take`), in that order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Targets {
    /// The old target it let go.
    pub released: Option<i32>,
    /// The new target it took.
    pub taken: Option<i32>,
}

/// A `MoveTo` that ended in the Artifact's update ([`Artifact::walk`]): its teardown stops the
/// unit (the caller's); the notify it posts (none for a quiet one) and the target the action let
/// go (slot 0x68 with none), the caller's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ended {
    /// The notify kind it posts, with the move action value.
    pub notify: Option<i32>,
    /// The target the action let go.
    pub released: Option<i32>,
}

/// One animal's Artifact action, as the checkpoint sees it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Artifact {
    /// 6 checks for a new owner; 10 waits for the timer.
    pub state: u8,
    /// `+0x44`, in seconds.
    pub timer: f32,
    /// Gaia waits half as long between checks.
    pub gaia: bool,
    /// The `MoveTo` sub-action of a move its player gave the animal, and the action's point
    /// (`+0x38`..`+0x40`) it walks to.
    pub move_to: Option<MoveTo>,
    /// The action's point (`+0x38`..`+0x40`): where its `MoveTo` walks, or its target's place.
    pub goal: Option<[f32; 3]>,
    /// The target of a move to an object (`+0x20`, its id at `+0x30`), −1 for none.
    pub target: i32,
    /// Its `MoveTo` to an object (`set_state(5)`): the goal, the range and that it is quiet
    /// (`+0x60` set: its end posts no notify); None for a `MoveTo` to the point.
    pub walk_to: Option<(Goal, f32)>,
}

/// The action type the checkpoint adds for an Artifact action.
pub const ARTIFACT_TYPE: i16 = 107;

const CHECK: u8 = 6;
const WAIT: u8 = 10;
/// Livestock's `MoveTo` to a town centre walks to its place less this on both axes
/// (`FUN_140aa0df0`, `DAT_142eae8d0`).
const TOWN_CENTRE_OFFSET: f32 = crate::image::artifact::TOWN_CENTRE_OFFSET;

impl Artifact {
    /// A new action, about to check for a new owner.
    #[must_use]
    pub const fn new(gaia: bool) -> Self {
        Self {
            state: CHECK,
            timer: 0.0,
            gaia,
            move_to: None,
            goal: None,
            target: -1,
            walk_to: None,
        }
    }

    /// Slot 0x190 and slot 0x288 on the animal with this as its top action (`FUN_14106d090`,
    /// `FUN_141082dc0`): no target (`+0x30` −1) and no point (`+0x38` −1). A herder waits for this
    /// before it slaughters.
    #[must_use]
    pub const fn idle(&self) -> bool {
        self.target == -1 && self.goal.is_none()
    }

    /// Slot 0x68, the base's (`FUN_140a7a230`, `+0x60` 0): a target other than `new` goes (let
    /// go), then `new` comes when it is one (not −1) in a state below 7 (`alive`, taken); a dead
    /// one leaves none.
    fn set_target(&mut self, new: i32, alive: bool) -> Targets {
        let mut t = Targets::default();
        if self.target == new {
            return t;
        }
        if self.target != -1 {
            t.released = Some(self.target);
        }
        if new != -1 && alive {
            self.target = new;
            t.taken = Some(new);
        } else {
            self.target = -1;
        }
        t
    }

    /// Slot 8 (`FUN_140aaca30`) on a move to the object `target` at `at`, in a state below 7
    /// (`alive`; the commands give no other): the old target goes, then the new one comes
    /// ([`Artifact::set_target`] with none, then with it), then `set_state(5)` (`0x140aace30`):
    /// its sub-actions go (`FUN_140a98620`: a walked `MoveTo`'s teardown stops the unit, the
    /// caller's), the point takes the target's place, and a quiet `MoveTo` (`FUN_140aa0df0`
    /// with `+0x60` 1) to the target starts, within 1 for livestock going to a building
    /// (`to_building`), else 0 (the unit's slot 0x268, 0); one of livestock to a town centre
    /// (master 109, `to_town_centre`) walks to the point 0.2 below the centre's on both axes
    /// instead, without the target. Then the action checks again (`set_state(6)`). Returns
    /// whether a `MoveTo` went and the targets let go and taken.
    pub fn take_object(
        &mut self,
        (target, alive): (i32, bool),
        at: [f32; 3],
        to_building: bool,
        to_town_centre: bool,
    ) -> (bool, Targets) {
        let released = self.set_target(-1, false).released;
        let taken = self.set_target(target, alive).taken;
        self.goal = Some(at);
        let range = if to_building { 1.0 } else { 0.0 };
        let goal = if to_town_centre {
            Goal::Point([
                at[0] - TOWN_CENTRE_OFFSET,
                at[1] - TOWN_CENTRE_OFFSET,
                at[2],
            ])
        } else {
            Goal::Target(target)
        };
        self.walk_to = Some((goal, range));
        let walked = self.move_to.replace(MoveTo::Start).is_some();
        self.state = CHECK;
        (walked, Targets { released, taken })
    }

    /// Slot 8 (`FUN_140aaca30`) on a move to a point, which the action takes: the point, then
    /// `set_state(4)` (`0x140aace30`): its sub-actions go (`FUN_140a98620`), a `MoveTo` to the point
    /// within 0 (`FUN_140aa0d30` with its task's sprite, walking; the sprite is not held) is added
    /// and starts (state 3 for an animal away from the point), and the action checks again
    /// (`set_state` 6). A move to an object (`set_state` 5) is not modelled. Returns whether a `MoveTo`
    /// went, whose teardown (`0x140aa1f30`) stops the unit (slot 0x5d0, the caller's).
    pub const fn take_point(&mut self, point: [f32; 3]) -> bool {
        self.goal = Some(point);
        self.walk_to = None;
        let walked = self.move_to.replace(MoveTo::Start).is_some();
        self.state = CHECK;
        walked
    }

    /// Slot 0x38 (`FUN_140aad1a0`), as the unit's stop (`FUN_14106fa60`) calls it (also before a
    /// new owner takes the animal): with a sub-action (`FUN_140a98730`) its sub-actions go, with
    /// the point (−1) and the target (slot 0x68 with none: let go); without one both stay. Then
    /// it checks again (`set_state` 6, which sets nothing else). Returns whether a `MoveTo` went,
    /// whose teardown (`0x140aa1f30`) stops the unit (slot 0x5d0, the caller's), and the target
    /// let go (the caller's release); the unit's standing sprite (slot 0x68 with the master's
    /// `+0x470`) is the caller's too.
    pub fn stop(&mut self) -> (bool, Option<i32>) {
        let walked = self.move_to.take().is_some();
        let mut released = None;
        if walked {
            self.walk_to = None;
            self.goal = None;
            released = self.set_target(-1, false).released;
        }
        self.state = CHECK;
        (walked, released)
    }

    /// The action's teardown (its destructor `FUN_140aab0d0` -> `FUN_140a73fc0`, as the action
    /// list's clearing or a dead object's runs it): the target goes (let go, `+0x60` 0); it holds
    /// no second target (`+0x28`); its sub-actions go, whose `MoveTo` teardown stops the unit.
    /// Returns whether a `MoveTo` went and the target let go, the caller's.
    pub fn tear_down(&mut self) -> (bool, Option<i32>) {
        let released = (self.target != -1).then_some(self.target);
        self.target = -1;
        let walked = self.move_to.take().is_some();
        self.walk_to = None;
        (walked, released)
    }

    /// One update (`0x140aad250`) with the tick's `dt_ms`. Its `MoveTo` goes first: it starts its
    /// walk, walks on, and once it ended (its update in state 1 returns 1) the sub-actions go, with
    /// the point and the target (slot 0x68 with none). Then the check (state 6): an animal of the
    /// combat type whose `+0x924` is 0 (`FUN_14107cc90`, `searches` false) never changes hands; any
    /// other asks `owner` (`FUN_140aabc40`) and returns the player that takes it, after its
    /// sub-actions went (`FUN_140a98620`), for the caller's conversion (`FUN_140aab570`). With no
    /// new owner it waits (`set_state` 10).
    pub fn update(
        &mut self,
        walk: Option<Walk<'_>>,
        dt_ms: u32,
        searches: bool,
        owner: &mut dyn FnMut() -> Result<Option<usize>, Unsupported>,
    ) -> Result<Option<(usize, bool)>, Unsupported> {
        let _ = self.walk(walk)?;
        self.step(dt_ms, searches, owner)
    }

    /// The update's first part: its `MoveTo` sub-action (`FUN_140a992b0`). Once the `MoveTo`
    /// ended ([`Ended`]): the notify kind the caller sends to the unit AI with the move action
    /// value (the `MoveTo` to the point is not quiet): its completion (0x1fa) from its update in
    /// state 1, or its failure (0x1f9) from state 13; a quiet one (to an object) posts none. A
    /// `MoveTo` whose motion step gave up fails (`set_state` 13: an animal never waits as a
    /// villager may); the failed one's update returns 3. On either code the Artifact drops it
    /// (`FUN_140a98620`: its teardown stops the unit, slot 0x5d0, the caller's), its point (−1)
    /// and its target (slot 0x68 with none: let go, the caller's release). A waiting `MoveTo` is
    /// not modelled.
    pub fn walk(&mut self, walk: Option<Walk<'_>>) -> Result<Option<Ended>, Unsupported> {
        let need = || Unsupported("a walking animal without its walk");
        match self.move_to {
            Some(MoveTo::Start) => {
                let (goal, range, quiet) = match self.walk_to {
                    Some((goal, range)) => (goal, range, true),
                    None => (
                        Goal::Point(
                            self.goal
                                .ok_or(Unsupported("an Artifact MoveTo without its point"))?,
                        ),
                        0.0,
                        false,
                    ),
                };
                self.move_to = Some(MoveTo::Moving);
                self.move_to = Some(start_walk(walk.ok_or_else(need)?, goal, range, quiet)?);
            }
            Some(MoveTo::Moving) => {
                let state = walk_or_give_up(walk.ok_or_else(need)?)?;
                self.move_to = Some(state.unwrap_or(MoveTo::Failed));
            }
            Some(MoveTo::Arrived) => {
                let (quiet, released) = self.end_walk();
                let notify = (!quiet).then_some(crate::unit_ai::ACTION_COMPLETED);
                return Ok(Some(Ended { notify, released }));
            }
            Some(MoveTo::Failed) => {
                let (quiet, released) = self.end_walk();
                let notify = (!quiet).then_some(crate::unit_ai::ACTION_FAILED);
                return Ok(Some(Ended { notify, released }));
            }
            Some(MoveTo::Waiting) => {
                return Err(Unsupported("an Artifact's waiting MoveTo (state 2)"));
            }
            None => {}
        }
        Ok(None)
    }

    /// The `MoveTo` ended (codes 1 to 5): it goes with the point and the target (slot 0x68 with
    /// none); true when it was quiet, and the target let go.
    fn end_walk(&mut self) -> (bool, Option<i32>) {
        self.move_to = None;
        self.goal = None;
        let released = self.set_target(-1, false).released;
        (self.walk_to.take().is_some(), released)
    }

    /// The update's second part: the state's step. A new owner comes with whether a `MoveTo`
    /// went (its teardown stops the unit, the caller's).
    pub fn step(
        &mut self,
        dt_ms: u32,
        searches: bool,
        owner: &mut dyn FnMut() -> Result<Option<usize>, Unsupported>,
    ) -> Result<Option<(usize, bool)>, Unsupported> {
        match self.state {
            CHECK => {
                if searches && let Some(new) = owner()? {
                    // its sub-actions go (FUN_140a98620) before the caller's conversion: the
                    // point and the target stay, so the conversion's stop ([`Artifact::stop`])
                    // finds no sub-action
                    let walked = self.move_to.take().is_some();
                    self.walk_to = None;
                    return Ok(Some((new, walked)));
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
        let _ = a.update(None, 50, true, &mut || Ok(None)).unwrap();
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
        let _ = a.update(None, 50, true, &mut || Ok(None)).unwrap();
        assert_eq!((a.state, a.timer, a.move_to, a.goal), (10, 1.0, None, None));
    }

    #[test]
    fn gaia_waits_half_a_second() {
        let mut a = Artifact::new(true);
        let _ = a.update(None, 50, true, &mut || Ok(None)).unwrap();
        assert_eq!(a.timer, 0.5);
    }

    #[test]
    fn the_wait_ends_when_the_timer_reaches_zero_then_one_check_follows() {
        let mut a = Artifact::new(false);
        let _ = a.update(None, 50, true, &mut || Ok(None)).unwrap();
        // 1.0 - 20 × 0.05 in f32 is -1.56e-7, so the 20th update ends the wait
        let waits = (0..30).take_while(|_| {
            let _ = a.update(None, 50, true, &mut || Ok(None)).unwrap();
            a.state == 10
        });
        assert_eq!(waits.count(), 19);
        assert_eq!(a.state, 6);
        let _ = a.update(None, 50, true, &mut || Ok(None)).unwrap();
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
        assert_eq!((found, a.state, asked), (Some((2, false)), 6, 1));
        // a combat-type animal whose +0x924 is 0 never asks
        let mut b = Artifact::new(true);
        let found = b.update(None, 50, false, &mut || Ok(Some(2))).unwrap();
        assert_eq!((found, b.state), (None, 10));
        // the conversion's stop: the check again, no MoveTo went
        assert_eq!(b.stop(), (false, None));
        assert_eq!(b.state, 6);
        // a walking animal's MoveTo goes (its teardown stops the unit), and its target is let go
        let mut walking = Artifact::new(false);
        let _ = walking.take_point([28.5, 94.5, 0.0]);
        walking.target = 9149;
        assert_eq!(walking.stop(), (true, Some(9149)));
        assert_eq!(
            (walking.move_to, walking.goal, walking.target),
            (None, None, -1)
        );
        // without a sub-action the point and the target stay (FUN_140a98730 false)
        let mut kept = Artifact::new(false);
        (kept.goal, kept.target, kept.state) = (Some([3.0, 4.0, 0.0]), 77, WAIT);
        assert_eq!(kept.stop(), (false, None));
        assert_eq!(
            (kept.goal, kept.target, kept.state),
            (Some([3.0, 4.0, 0.0]), 77, CHECK)
        );
    }

    /// The check that finds a new owner drops its walking `MoveTo` (`FUN_140a98620`, the unit
    /// stopped by the caller) and keeps its point and target, which the conversion's stop then
    /// leaves (no sub-action).
    #[test]
    fn a_walking_animal_taken_over_keeps_its_point_and_target() {
        let mut a = Artifact::new(true);
        a.goal = Some([5.0, 6.0, 0.0]);
        a.target = 9149;
        a.walk_to = Some((Goal::Target(9149), 1.0));
        a.move_to = Some(MoveTo::Moving);
        let found = a.step(50, true, &mut || Ok(Some(1))).unwrap();
        assert_eq!(found, Some((1, true)));
        assert_eq!(
            (a.move_to, a.walk_to, a.goal, a.target),
            (None, None, Some([5.0, 6.0, 0.0]), 9149)
        );
        assert_eq!(a.stop(), (false, None));
        assert_eq!(a.target, 9149);
    }

    /// Slot 8 on an object (`FUN_140aaca30`): the old target is let go, then the new one taken
    /// (slot 0x68 twice); the same object is let go and taken again; a dead one is not taken.
    /// The teardown lets the target go and stops a walk.
    #[test]
    fn a_move_to_an_object_lets_the_old_target_go_and_takes_the_new() {
        let mut a = Artifact::new(false);
        let (walked, t) = a.take_object((9149, true), [10.0, 12.0, 0.0], true, false);
        assert_eq!(
            (walked, t),
            (
                false,
                Targets {
                    released: None,
                    taken: Some(9149)
                }
            )
        );
        assert_eq!((a.target, a.move_to), (9149, Some(MoveTo::Start)));
        let (walked, t) = a.take_object((9149, true), [10.0, 12.0, 0.0], true, false);
        assert!(walked);
        assert_eq!(
            t,
            Targets {
                released: Some(9149),
                taken: Some(9149)
            }
        );
        let (_, t) = a.take_object((300, false), [1.0, 2.0, 0.0], false, false);
        assert_eq!(
            t,
            Targets {
                released: Some(9149),
                taken: None
            }
        );
        assert_eq!(a.target, -1);
        a.target = 9149;
        assert_eq!(a.tear_down(), (true, Some(9149)));
        assert_eq!((a.target, a.move_to, a.walk_to), (-1, None, None));
        assert_eq!(a.tear_down(), (false, None));
    }

    /// A `MoveTo` that ended goes with the point and lets the target go (slot 0x68 with none).
    #[test]
    fn an_ended_walk_lets_the_target_go() {
        let mut a = Artifact::new(false);
        a.target = 9149;
        a.goal = Some([1.0, 1.0, 0.0]);
        a.walk_to = Some((Goal::Target(9149), 1.0));
        a.move_to = Some(MoveTo::Arrived);
        let quiet = Ended {
            notify: None,
            released: Some(9149),
        };
        assert_eq!(a.walk(None), Ok(Some(quiet)));
        a.target = 77;
        a.move_to = Some(MoveTo::Failed);
        let failed = Ended {
            notify: Some(crate::unit_ai::ACTION_FAILED),
            released: Some(77),
        };
        assert_eq!(a.walk(None), Ok(Some(failed)));
        assert_eq!(a.walk(None), Ok(None));
    }
}
