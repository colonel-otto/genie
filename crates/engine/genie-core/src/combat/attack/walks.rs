//! The `MoveTo`s the Attack's `set_state` makes (`0x140a820c0`, cases 4, 5 and 11) and the update
//! of its sub-action list (`FUN_140a992b0`).

use crate::GenieError;
use crate::combat::attack::{ATTACK_WITHOUT_TARGET, Attack, AttackReads, GOTO_POINT};
use crate::movement::move_to::{GiveUp, MoveTo, MoveToAction, NO_TARGET_GAP, Shown};
use crate::movement::{Me, PathAsk};
use crate::objects::unit::Unit;
use crate::unit_ai::Inbox;
use crate::world::step::Step;

/// A sub-action list's code that a quiet `MoveTo` does not give: 4 is its target let go when it
/// is not quiet.
pub(super) const QUIET_CODE: GenieError =
    GenieError::unreachable("an Attack's quiet MoveTo's code that only a loud one gives");

/// The unit's top speed (slot 0x208), which a `MoveTo`'s start reads.
///
/// # Errors
///
/// A unit that does not move.
pub(crate) fn speed_of(unit: &Unit) -> Result<f32, GenieError> {
    Ok(unit
        .body
        .mover()
        .ok_or(crate::world::ATTACKER_UNMOVING)?
        .stats
        .speed)
}

impl Attack {
    /// `set_state(4)`'s `MoveTo` to the target object (`FUN_140aa0df0` with the Attack's range and
    /// walking sprite, quiet through `FUN_140a7a170` with 1, its `+0x61` the Attack's), as the
    /// action list adds and starts it ([`MoveToAction::added`], which reads the unit's object
    /// state `unit_state` and its top speed): `at` is the target's place and
    /// `gap` the unit's from the target's footprint (`FUN_14118f680`). A unit within 0.01 of it
    /// is there already, state 1; else state 3, or 13 for a unit without speed. None for a unit
    /// whose state is 3 or more: the list deletes it. The timer is 0. True when it started
    /// arrived.
    ///
    /// # Errors
    ///
    /// An Attack without a target.
    pub(super) fn walk_to_target(
        &mut self,
        at: [f32; 3],
        gap: f32,
        unit_state: u8,
        speed: f32,
    ) -> Result<bool, GenieError> {
        if self.target == -1 {
            return Err(ATTACK_WITHOUT_TARGET.with("state", self.state));
        }
        let action = MoveToAction {
            target: self.target,
            on_target_moved: self.on_target_moved,
            ..MoveToAction::sub_to_point(at, self.range, self.arms.move_sprite)
        };
        self.move_to = action.added(unit_state, gap, speed);
        self.timer = 0.0;
        Ok(self.move_to.is_some_and(|m| m.arrived()))
    }

    /// The `MoveTo` to a point of states 5 and 11 (`FUN_140aa0d30` to where the target was seen,
    /// `+0xa4`..`+0xac`, with the Attack's walking sprite, quiet, its `+0x61` the Attack's), as
    /// the action list adds and starts it: state 5 walks within the range, state 11 onto the
    /// point. The point's gap is [`NO_TARGET_GAP`], so state 3, or 13 for a unit without speed.
    /// None for a unit whose state is 3 or more. The timer is 0.
    pub(super) const fn walk_to_seen(&mut self, unit_state: u8, speed: f32) {
        let range = if self.state == GOTO_POINT {
            0.0
        } else {
            self.range
        };
        let action = MoveToAction {
            on_target_moved: self.on_target_moved,
            ..MoveToAction::sub_to_point(self.seen, range, self.arms.move_sprite)
        };
        self.move_to = action.added(unit_state, NO_TARGET_GAP, speed);
        self.timer = 0.0;
    }

    /// The Attack's sub-action list's update (`FUN_140a992b0`: its first action's, the quiet
    /// `MoveTo`'s, [`MoveToAction::sub_update`]) and its code, which the state's case reads: 0
    /// while it walks or waits, 1 once it ended, 3 once it failed, 5 once its target was let go;
    /// 1 for an empty list. The start shows the Attack's walking sprite (the unit's standing one
    /// for none), the motion step gives up as `gives_up` says, and a start or a move posts the
    /// units in the unit's way to its unit AI at once.
    ///
    /// # Errors
    ///
    /// The walk's and the notifies'.
    pub(super) fn update_list(
        &mut self,
        c: &mut Step<'_>,
        unit: &mut Unit,
        me: Me,
        r: &AttackReads,
        gives_up: GiveUp,
    ) -> Result<u8, GenieError> {
        let Some(move_to) = &mut self.move_to else {
            return Ok(1);
        };
        let state = move_to.state;
        let held = c.w.object_state(move_to.target);
        let (mut walk, beside) = c.w.walk_beside(c.pace(), &mut unit.body, me);
        let shown = (state == MoveTo::Start).then_some(Shown {
            sprite: &mut unit.sprite,
            rng: beside.sprite_rng,
            graphic: move_to.sprite.or(r.standing),
        });
        let mut inbox = Inbox::of(&mut unit.ai, unit.id, unit.life.state);
        let ask = PathAsk::default();
        let code = move_to.sub_update(&mut walk, ask, gives_up, held, shown, &mut inbox)?;
        if matches!(state, MoveTo::Start | MoveTo::Moving) {
            unit.post_in_the_way()?;
        }
        Ok(code)
    }

    /// The sub-action list's `MoveTo` goes: the list deletes it and its destructor
    /// (`0x140aa1f30`) stops the unit.
    ///
    /// # Errors
    ///
    /// A unit that does not move.
    pub(super) fn delete_move_to(&mut self, unit: &mut Unit) -> Result<(), GenieError> {
        if self.move_to.take().is_some() {
            unit.body
                .mover_mut()
                .ok_or(crate::world::ATTACKER_UNMOVING)?
                .stop()?;
        }
        Ok(())
    }
}
