//! The Artifact action (type 107): the default task of livestock, which keeps checking whether a
//! unit of another player is close enough to take the animal over (the search is the caller's,
//! [`crate::map::spatial::Spatial::artifact_owner`]).
//!
//! 158041: `TRIBE_Action_Artifact`, vtable `0x142ed6520`; update `0x140aad250` (slot 6), `set_state`
//! `0x140aace30` (slot 16). The .dat gives Goat, Goose, Cow and Sheep the default task 0, a task with
//! action type 107. An idle animal holds this action as its top action, so checkpoint word 9 counts
//! `107 + state` for it.

use crate::GenieError;
use crate::map::spatial::Spatial;
use crate::movement::Me;
use crate::movement::move_to::{Goal, MoveTo, Walk, start_walk_or_give_up, walk_or_give_up};
use crate::objects::unit::Unit;
use crate::ownership::capture::Captured;
use crate::world::context::Context;
use crate::world::others::Others;

/// What the Artifact's slot 0x68 (the base's `FUN_140a7a230`) did to its targets, for the
/// caller: the action is not quiet (`+0x60` 0, its constructors' `FUN_140a73e90` with 1), so the
/// old target, when the world holds it, lets the unit go (its slot 0x180, `World::release`) and
/// a new one in a state below 7 takes it (its slot 0x178, `World::take`), in that order.
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

/// What the Artifact's state step asks of its unit ([`Artifact::step`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handover {
    /// The check found player `new` to take the animal over (the caller's conversion,
    /// `FUN_140aab570`), its sub-actions gone ([`Artifact::drop_sub_actions`]).
    Taken {
        /// The player that takes it.
        new: usize,
        /// The sub-actions that went.
        gone: SubActionsGone,
    },
    /// State 100: the unit's stop (slot 0x148), then its ownership change to Gaia (slot 0x70),
    /// the caller's; the action checks again (`set_state(6)`).
    ToGaia,
}

/// What the deletion of the Artifact's sub-actions (`FUN_140a98620`) leaves to the caller
/// ([`Artifact::drop_sub_actions`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SubActionsGone {
    /// A walking `MoveTo` went, the Artifact's own or the Enter's: its teardown (`0x140aa1f30`)
    /// stops the unit (slot 0x5d0).
    pub stops: bool,
    /// The building or transport the Enter's teardown let go (slot 0x180).
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
    /// The Enter an enter command made its sub-action (`FUN_1410847e0` under an Artifact on
    /// top), which its update runs first ([`Artifact::hold_enter`]).
    pub enter: Option<crate::garrison::EnterAction>,
}

/// The action type the checkpoint adds for an Artifact action.
pub const ARTIFACT_TYPE: i16 = 107;

/// The state that checks for a new owner (6).
pub(crate) const CHECK: u8 = 6;
const WAIT: u8 = 10;
/// The state that gives the animal to Gaia (`0x140aad250`'s case `'d'`).
pub(crate) const TO_GAIA: u8 = 100;
/// Livestock's `MoveTo` to a town centre walks to its place less this on both axes
/// (`FUN_140aa0df0`, `DAT_142eae8d0`).
const TOWN_CENTRE_OFFSET: f32 = crate::math::image::artifact::TOWN_CENTRE_OFFSET;

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
            enter: None,
        }
    }

    /// Slot 0x68 with none: the target goes; the one let go, if any.
    pub fn drop_target(&mut self) -> Option<i32> {
        self.set_target(-1, false).released
    }

    /// `FUN_140a98620` on the sub-actions (one at most): a `MoveTo` goes with its goal, whose
    /// teardown stops a walking unit; an Enter goes with its teardown (`FUN_140a85bf0` ->
    /// `FUN_140a73fc0`): its building or transport is let go (slot 0x180), and its own `MoveTo`,
    /// if it walks, goes too. The stop and the release are the caller's.
    pub fn drop_sub_actions(&mut self) -> SubActionsGone {
        let walked = self.move_to.take().is_some();
        self.walk_to = None;
        let enter = self.enter.take();
        SubActionsGone {
            stops: walked || enter.is_some_and(|e| e.move_to.is_some()),
            released: enter.map(|e| e.target),
        }
    }

    /// The enter command's Enter as the sub-action (`FUN_1410847e0` with an Artifact on top:
    /// `FUN_140a98e20` pops the sub-action it held, [`Artifact::drop_sub_actions`], then
    /// `FUN_140a977e0` takes the Enter). What went, the caller's.
    pub fn hold_enter(&mut self, enter: crate::garrison::EnterAction) -> SubActionsGone {
        let gone = self.drop_sub_actions();
        self.enter = Some(enter);
        gone
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
    /// its sub-actions go ([`Artifact::drop_sub_actions`]), the point takes the target's place,
    /// and a quiet `MoveTo` (`FUN_140aa0df0`
    /// with `+0x60` 1) to the target starts, within 1 for livestock going to a building
    /// (`to_building`), else 0 (the unit's slot 0x268, 0); one of livestock to a town centre
    /// (master 109, `to_town_centre`) walks to the point 0.2 below the centre's on both axes
    /// instead, without the target. Then the action checks again (`set_state(6)`). Returns the
    /// targets let go and taken, then the sub-actions that went.
    pub fn take_object(
        &mut self,
        (target, alive): (i32, bool),
        at: [f32; 3],
        to_building: bool,
        to_town_centre: bool,
    ) -> (Targets, SubActionsGone) {
        let released = self.set_target(-1, false).released;
        let taken = self.set_target(target, alive).taken;
        let gone = self.drop_sub_actions();
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
        self.move_to = Some(MoveTo::Start);
        self.state = CHECK;
        (Targets { released, taken }, gone)
    }

    /// Slot 8 (`FUN_140aaca30`) on a move to a point, which the action takes: the point, then
    /// `set_state(4)` (`0x140aace30`): its sub-actions go ([`Artifact::drop_sub_actions`]), a
    /// `MoveTo` to the point within 0 (`FUN_140aa0d30` with its task's sprite, walking; the
    /// sprite is not held) is added and starts (state 3 for an animal away from the point), and
    /// the action checks again (`set_state` 6). Returns the sub-actions that went.
    pub fn take_point(&mut self, point: [f32; 3]) -> SubActionsGone {
        self.goal = Some(point);
        let gone = self.drop_sub_actions();
        self.move_to = Some(MoveTo::Start);
        self.state = CHECK;
        gone
    }

    /// Slot 0x38 (`FUN_140aad1a0`), as the unit's stop (`FUN_14106fa60`) calls it (also before a
    /// new owner takes the animal): with a sub-action (`FUN_140a98730`) its sub-actions go
    /// ([`Artifact::drop_sub_actions`]), with the point (−1) and the target (slot 0x68 with none:
    /// let go); without one both stay. Then it checks again (`set_state` 6, which sets nothing
    /// else). Returns the sub-actions that went, then the target let go (the caller's release,
    /// after the Enter's); the unit's standing sprite (slot 0x68 with the master's `+0x470`) is
    /// the caller's too.
    pub fn stop(&mut self) -> (SubActionsGone, Option<i32>) {
        let mut gone = SubActionsGone::default();
        let mut released = None;
        if self.move_to.is_some() || self.enter.is_some() {
            gone = self.drop_sub_actions();
            self.goal = None;
            released = self.set_target(-1, false).released;
        }
        self.state = CHECK;
        (gone, released)
    }

    /// The action's teardown (its destructor `FUN_140aab0d0` -> `FUN_140a73fc0`, as the action
    /// list's clearing or a dead object's runs it): the target goes (let go, `+0x60` 0); it holds
    /// no second target (`+0x28`); its sub-actions go ([`Artifact::drop_sub_actions`]). Returns
    /// the target let go, then the sub-actions that went, the caller's.
    pub fn tear_down(&mut self) -> (Option<i32>, SubActionsGone) {
        let released = (self.target != -1).then_some(self.target);
        self.target = -1;
        (released, self.drop_sub_actions())
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
        owner: &mut dyn FnMut() -> Result<Option<usize>, GenieError>,
    ) -> Result<Option<Handover>, GenieError> {
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
    pub fn walk(&mut self, walk: Option<Walk<'_>>) -> Result<Option<Ended>, GenieError> {
        let need = || GenieError::unsupported("a walking animal without its walk");
        match self.move_to {
            Some(MoveTo::Start) => {
                let (goal, range, quiet) = match self.walk_to {
                    Some((goal, range)) => (goal, range, true),
                    None => (
                        Goal::Point(self.goal.ok_or(GenieError::unsupported(
                            "an Artifact MoveTo without its point",
                        ))?),
                        0.0,
                        false,
                    ),
                };
                self.move_to = Some(MoveTo::Moving);
                // a motion step that gives up in the start's update fails the MoveTo, as in
                // its moving state
                let started = start_walk_or_give_up(walk.ok_or_else(need)?, goal, range, quiet)?;
                self.move_to = Some(started.unwrap_or(MoveTo::Failed));
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
                return Err(GenieError::unsupported(
                    "an Artifact's waiting MoveTo (state 2)",
                ));
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

    /// The update's second part: the state's step ([`Handover`]): the check's new owner, with
    /// whether a `MoveTo` went (its teardown stops the unit, the caller's), or state 100's
    /// handover to Gaia (the action checks again; the stop and the change are the caller's).
    pub fn step(
        &mut self,
        dt_ms: u32,
        searches: bool,
        owner: &mut dyn FnMut() -> Result<Option<usize>, GenieError>,
    ) -> Result<Option<Handover>, GenieError> {
        match self.state {
            CHECK => {
                if searches && let Some(new) = owner()? {
                    // its sub-actions go (FUN_140a98620) before the caller's conversion: the
                    // point and the target stay, so the conversion's stop ([`Artifact::stop`])
                    // finds no sub-action
                    let gone = self.drop_sub_actions();
                    return Ok(Some(Handover::Taken { new, gone }));
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
            TO_GAIA => {
                self.state = CHECK;
                return Ok(Some(Handover::ToGaia));
            }
            // its states 4 and 5 (`set_state`'s walks) pass on to 6 at once; the update has no
            // case for any other
            _ => {}
        }
        Ok(None)
    }
}

impl Unit {
    /// The Artifact's part of the update (`0x140aad250`): its Enter sub-action first
    /// ([`Unit::artifact_enter_part`]), then its walk and the check for a new owner
    /// ([`Unit::artifact_walk_and_check`]), then the handover it found
    /// ([`Unit::artifact_handover`]). In the first tick it waits.
    pub(crate) fn artifact_part(
        &mut self,
        c: &mut Context<'_>,
        others: &mut Others<'_>,
        me: Me,
    ) -> Result<Option<Captured>, GenieError> {
        let by = self.releaser(c);
        if c.tick <= 1 {
            return Ok(None);
        }
        self.artifact_enter_part(c, others, me, by)?;
        let new = self.artifact_walk_and_check(c, others, me, by)?;
        self.artifact_handover(c, others, new)
    }

    /// The Artifact's Enter sub-action (an enter command's, `FUN_1410847e0`) updates first
    /// (`FUN_140a992b0`) as the unit's own Enter does, taken as the unit's meanwhile. Once it
    /// ended (1 to 5: the failed one's 3, the list removing it) the sub-actions go, the point is
    /// −1 and the target goes (slot 0x68 with none, released by `by`); an Enter that got the
    /// unit in stays, and the inside update of the next update deletes the Artifact.
    pub(crate) fn artifact_enter_part(
        &mut self,
        c: &mut Context<'_>,
        others: &mut Others<'_>,
        me: Me,
        by: crate::objects::release::Releaser,
    ) -> Result<(), GenieError> {
        let Some(e) = self.actions.artifact_mut().and_then(|a| a.enter.take()) else {
            return Ok(());
        };
        let own = self.actions.take_enter();
        self.actions.set_enter(Some(e));
        let updated = self.enter_update(c, others, me);
        let after = self.actions.take_enter();
        self.actions.set_enter(own);
        updated?;
        if let Some(art) = self.actions.artifact_mut() {
            if let Some(e) = after {
                art.enter = Some(e);
            } else {
                art.goal = None;
                if let Some(t) = art.drop_target() {
                    crate::world::lets::release(c, others, t, by)?;
                }
            }
        }
        Ok(())
    }

    /// The Artifact's walk and its check for a new owner: an ended `MoveTo` (codes 1 to 5) goes
    /// (`FUN_140a98620`: its teardown stops the unit, quiet or not) and the action lets its
    /// target go; the check (`FUN_14107cc90`: an object of the combat types whose `+0x924` is 0
    /// never changes hands) runs [`Spatial::artifact_owner`]. A handover's check lets its
    /// sub-actions go (an Enter's teardown lets its building go; a walking `MoveTo`'s, the
    /// Artifact's own or the Enter's, stops the unit); then the ended `MoveTo`'s notify. The
    /// handover found, if any; none without an Artifact.
    pub(crate) fn artifact_walk_and_check(
        &mut self,
        c: &mut Context<'_>,
        others: &mut Others<'_>,
        me: Me,
        by: crate::objects::release::Releaser,
    ) -> Result<Option<Handover>, GenieError> {
        let Some(artifact) = self.actions.artifact_mut() else {
            return Ok(None);
        };
        let dt = c.dt;
        let ended = artifact.walk(Some(c.walk(self.body.mover_mut(), me)))?;
        if ended.is_some()
            && let Some(m) = self.body.mover_mut()
        {
            m.stop()?;
        }
        if let Some(t) = ended.and_then(|e| e.released) {
            crate::world::lets::release(c, others, t, by)?;
        }
        let capture = c
            .spatial
            .sight
            .as_ref()
            .and_then(|v| v.seer(self.id))
            .map(|s| s.capture);
        let searches = !(self.combat && capture == Some(0));
        let spatial: &Spatial = c.spatial;
        let (attribute_97, game_status) = (c.attribute_97, c.game_status);
        let id = self.id;
        let new = artifact.step(dt, searches, &mut || {
            spatial.artifact_owner(id, attribute_97, game_status)
        })?;
        if let Some(Handover::Taken { gone, .. }) = new {
            if let Some(t) = gone.released {
                crate::world::lets::release(c, others, t, by)?;
            }
            if gone.stops
                && let Some(m) = self.body.mover_mut()
            {
                m.stop()?;
            }
        }
        if let Some(kind) = ended.and_then(|e| e.notify) {
            self.post_own(kind, [crate::unit_ai::MOVE_ACTION, 0, 0])?;
        }
        Ok(new)
    }

    /// The handover the Artifact's check found: to a new owner its capture
    /// ([`Unit::capture`]); state 100's to Gaia the unit's stop (slot 0x148, [`Unit::capture_stop`])
    /// and slot 0x70 to Gaia, reported when the unit or a building changed hands. None changes
    /// nothing.
    pub(crate) fn artifact_handover(
        &mut self,
        c: &mut Context<'_>,
        others: &mut Others<'_>,
        new: Option<Handover>,
    ) -> Result<Option<Captured>, GenieError> {
        match new {
            Some(Handover::Taken { new, .. }) => {
                let (main, also) = self.capture(c, others, new)?;
                Ok(Some(Captured {
                    new,
                    main,
                    also,
                    building: std::mem::take(&mut c.building_handover),
                }))
            }
            Some(Handover::ToGaia) => {
                let class = self.placed_class(c)?;
                self.capture_stop(c, others, class)?;
                let main = self.change_owner(c, 0)?;
                let building = std::mem::take(&mut c.building_handover);
                Ok((main || building).then(|| Captured {
                    new: 0,
                    main,
                    also: Vec::new(),
                    building,
                }))
            }
            None => Ok(None),
        }
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
        assert_eq!(a.take_point([28.5, 93.5, 0.0]), SubActionsGone::default());
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
        assert_eq!(
            walking.take_point([28.5, 94.5, 0.0]),
            SubActionsGone {
                stops: true,
                released: None
            }
        );
        assert_eq!(walking.move_to, Some(MoveTo::Start));
        // its walk needs the animal's movement state
        assert!(a.update(None, 50, true, &mut || Ok(None)).is_err());
        // a MoveTo that ended goes with the point, then the check waits again
        a.move_to = Some(MoveTo::Arrived);
        let _ = a.update(None, 50, true, &mut || Ok(None)).unwrap();
        assert_eq!((a.state, a.timer, a.move_to, a.goal), (10, 1.0, None, None));
    }

    /// `set_state(4)` and `set_state(5)` (`0x140aace30`) delete every sub-action
    /// (`FUN_140a98620`), the Enter too: its transport is let go and its walking `MoveTo` stops
    /// the unit (429750109's livestock 11962 walking into the transport 11970, then MOVE to
    /// (32.854168, 120.729164) in update 44: the new `MoveTo` alone holds it).
    #[test]
    fn a_move_deletes_the_held_enter_letting_its_transport_go() {
        let walking_enter = || {
            let mut e = crate::garrison::EnterAction::new(11970, [34.125, 123.333, 0.0], 0.0, 20);
            e.move_to = Some(MoveTo::Moving);
            e
        };
        let by_enter = SubActionsGone {
            stops: true,
            released: Some(11970),
        };
        let mut a = Artifact::new(false);
        a.enter = Some(walking_enter());
        assert_eq!(a.take_point([32.854_168, 120.729_164, 0.0]), by_enter);
        assert_eq!(
            (a.enter, a.move_to, a.goal),
            (
                None,
                Some(MoveTo::Start),
                Some([32.854_168, 120.729_164, 0.0])
            )
        );
        // to an object: the old target goes and the new one comes first, then the Enter
        let mut b = Artifact::new(false);
        b.enter = Some(walking_enter());
        let (targets, gone) = b.take_object((9149, true), [5.0, 6.0, 0.0], true, false);
        assert_eq!(
            (targets.released, targets.taken, gone),
            (None, Some(9149), by_enter)
        );
        assert_eq!((b.enter, b.move_to), (None, Some(MoveTo::Start)));
        // an Enter that does not walk stops nothing
        let mut c = Artifact::new(false);
        c.enter = Some(crate::garrison::EnterAction::new(
            11970,
            [1.0, 2.0, 0.0],
            0.0,
            20,
        ));
        assert_eq!(
            c.take_point([3.0, 4.0, 0.0]),
            SubActionsGone {
                stops: false,
                released: Some(11970)
            }
        );
    }

    /// Slot 0x38 (`FUN_140aad1a0`) with an Enter as the sub-action (`FUN_140a98730` true): the
    /// Enter goes (its transport let go, its walk stopped), then the point and the target; the
    /// teardown (`FUN_140a73fc0`) lets the target go before the Enter's.
    #[test]
    fn the_stop_and_the_teardown_delete_the_held_enter() {
        let mut e = crate::garrison::EnterAction::new(11970, [1.0, 2.0, 0.0], 0.0, 20);
        e.move_to = Some(MoveTo::Moving);
        let by_enter = SubActionsGone {
            stops: true,
            released: Some(11970),
        };
        let mut a = Artifact::new(false);
        (a.enter, a.goal, a.target, a.state) = (Some(e), Some([1.0, 2.0, 0.0]), 77, WAIT);
        assert_eq!(a.stop(), (by_enter, Some(77)));
        assert_eq!(
            (a.enter, a.goal, a.target, a.state),
            (None, None, -1, CHECK)
        );
        let mut b = Artifact::new(false);
        (b.enter, b.target) = (Some(e), 77);
        assert_eq!(b.tear_down(), (Some(77), by_enter));
        assert_eq!((b.enter, b.target), (None, -1));
        // the check that finds an owner deletes it too
        let mut c = Artifact::new(false);
        c.enter = Some(e);
        let found = c.step(50, true, &mut || Ok(Some(2))).unwrap();
        assert_eq!(
            found,
            Some(Handover::Taken {
                new: 2,
                gone: by_enter
            })
        );
        assert_eq!(c.enter, None);
    }

    /// `FUN_1410847e0` under an Artifact: `FUN_140a98e20` pops the sub-action the enter command
    /// replaces. An Enter goes with its teardown (`FUN_140a85bf0` -> `FUN_140a73fc0`): its
    /// building or transport is let go (slot 0x180), and its walking `MoveTo`'s teardown stops
    /// the unit (430061417's pig 7920 in update 32: INTERACT on the transport 7934 again, the
    /// unit's speed and acceleration 0, so the next velocity step does not move it).
    #[test]
    fn an_enter_command_replaces_the_held_enter_letting_its_target_go_and_stopping_its_walk() {
        let enter = |target| crate::garrison::EnterAction::new(target, [1.0, 2.0, 0.0], 0.0, 20);
        let gone = |stops, released| SubActionsGone { stops, released };
        let mut a = Artifact::new(false);
        // nothing held: nothing to let go, nothing walking
        assert_eq!(a.hold_enter(enter(7934)), gone(false, None));
        // the held Enter walks: its MoveTo's teardown stops the unit
        a.enter.as_mut().unwrap().move_to = Some(MoveTo::Moving);
        assert_eq!(a.hold_enter(enter(7934)), gone(true, Some(7934)));
        assert_eq!(a.enter.map(|e| (e.target, e.move_to)), Some((7934, None)));
        // an Enter not walking only lets its target go
        assert_eq!(a.hold_enter(enter(7935)), gone(false, Some(7934)));
        // the Artifact's own MoveTo goes with its goal and stops the unit
        a.enter = None;
        a.move_to = Some(MoveTo::Moving);
        a.walk_to = Some((Goal::Point([3.0, 4.0, 0.0]), 0.0));
        assert_eq!(a.hold_enter(enter(7934)), gone(true, None));
        assert_eq!((a.move_to, a.walk_to), (None, None));
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
        assert_eq!(
            (found, a.state, asked),
            (
                Some(Handover::Taken {
                    new: 2,
                    gone: SubActionsGone::default()
                }),
                6,
                1
            )
        );
        // a combat-type animal whose +0x924 is 0 never asks
        let mut b = Artifact::new(true);
        let found = b.update(None, 50, false, &mut || Ok(Some(2))).unwrap();
        assert_eq!((found, b.state), (None, 10));
        // the conversion's stop: the check again, no MoveTo went
        assert_eq!(b.stop(), (SubActionsGone::default(), None));
        assert_eq!(b.state, 6);
        // a walking animal's MoveTo goes (its teardown stops the unit), and its target is let go
        let mut walking = Artifact::new(false);
        let _ = walking.take_point([28.5, 94.5, 0.0]);
        walking.target = 9149;
        assert_eq!(
            walking.stop(),
            (
                SubActionsGone {
                    stops: true,
                    released: None
                },
                Some(9149)
            )
        );
        assert_eq!(
            (walking.move_to, walking.goal, walking.target),
            (None, None, -1)
        );
        // without a sub-action the point and the target stay (FUN_140a98730 false)
        let mut kept = Artifact::new(false);
        (kept.goal, kept.target, kept.state) = (Some([3.0, 4.0, 0.0]), 77, WAIT);
        assert_eq!(kept.stop(), (SubActionsGone::default(), None));
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
        assert_eq!(
            found,
            Some(Handover::Taken {
                new: 1,
                gone: SubActionsGone {
                    stops: true,
                    released: None
                }
            })
        );
        assert_eq!(
            (a.move_to, a.walk_to, a.goal, a.target),
            (None, None, Some([5.0, 6.0, 0.0]), 9149)
        );
        assert_eq!(a.stop(), (SubActionsGone::default(), None));
        assert_eq!(a.target, 9149);
    }

    /// State 100 (`0x140aad250`'s case `'d'`): the animal goes to Gaia, the caller's, and the
    /// action checks again; it asks for no owner.
    #[test]
    fn state_100_hands_the_animal_to_gaia() {
        let mut a = Artifact {
            state: TO_GAIA,
            ..Artifact::new(false)
        };
        let found = a
            .step(50, true, &mut || Err(GenieError::unsupported("no search")))
            .unwrap();
        assert_eq!((found, a.state), (Some(Handover::ToGaia), CHECK));
    }

    /// Slot 8 on an object (`FUN_140aaca30`): the old target is let go, then the new one taken
    /// (slot 0x68 twice); the same object is let go and taken again; a dead one is not taken.
    /// The teardown lets the target go and stops a walk.
    #[test]
    fn a_move_to_an_object_lets_the_old_target_go_and_takes_the_new() {
        let mut a = Artifact::new(false);
        let (t, gone) = a.take_object((9149, true), [10.0, 12.0, 0.0], true, false);
        assert_eq!(
            (t, gone),
            (
                Targets {
                    released: None,
                    taken: Some(9149)
                },
                SubActionsGone::default()
            )
        );
        assert_eq!((a.target, a.move_to), (9149, Some(MoveTo::Start)));
        let (t, gone) = a.take_object((9149, true), [10.0, 12.0, 0.0], true, false);
        assert!(gone.stops);
        assert_eq!(
            t,
            Targets {
                released: Some(9149),
                taken: Some(9149)
            }
        );
        let (t, _) = a.take_object((300, false), [1.0, 2.0, 0.0], false, false);
        assert_eq!(
            t,
            Targets {
                released: Some(9149),
                taken: None
            }
        );
        assert_eq!(a.target, -1);
        a.target = 9149;
        assert_eq!(
            a.tear_down(),
            (
                Some(9149),
                SubActionsGone {
                    stops: true,
                    released: None
                }
            )
        );
        assert_eq!((a.target, a.move_to, a.walk_to), (-1, None, None));
        assert_eq!(a.tear_down(), (None, SubActionsGone::default()));
    }

    /// The motion step gives up in the update the start's `set_state(4)` runs (426321525's
    /// livestock in update 2656; a unit without speed here): the `MoveTo` fails, as in its moving
    /// state, and the next update ends it with its failure.
    #[test]
    fn a_walk_the_motion_step_gives_up_at_its_start_fails() {
        let mut a = Artifact::new(false);
        let _ = a.take_point([28.5, 93.5, 0.0]);
        let mut mover = crate::gathering::hunt::tests::walker();
        mover.stats.speed = 0.0;
        let mut world = Spatial::new([120, 120]);
        let walk = Walk {
            mover: Some(&mut mover),
            me: Me {
                id: 9517,
                position: [20.5, 90.5, 0.0],
                group: -1,
                action_state: 0,
            },
            dt: 0.05,
            world: &mut world,
            plan: None,
        };
        assert_eq!(a.walk(Some(walk)), Ok(None));
        assert_eq!(a.move_to, Some(MoveTo::Failed));
        let failed = Ended {
            notify: Some(crate::unit_ai::ACTION_FAILED),
            released: None,
        };
        assert_eq!(a.walk(None), Ok(Some(failed)));
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
