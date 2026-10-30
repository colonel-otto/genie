//! The `MoveTo`s the Hunt's `set_state` makes (`0x140ac60e0`) and the update of its sub-action
//! list (`FUN_140a992b0`).

use crate::GenieError;
use crate::gathering::gather::Walks;
use crate::gathering::hunt::{HuntAction, HuntReads};
use crate::movement::move_to::{
    GiveUp, MoveTo, MoveToAction, NO_TARGET_GAP, Shown, SpriteReads, TargetMoved,
};
use crate::movement::{Me, Mover, PathAsk};
use crate::objects::sprite::Graphic;
use crate::objects::unit::Unit;
use crate::unit_ai::Inbox;
use crate::world::step::Step;

/// A sub-action list's code that a quiet `MoveTo` does not give: 4 is its target let go when it
/// is not quiet, and no code is above 5.
pub(super) const QUIET_CODE: GenieError =
    GenieError::unreachable("a quiet sub-action MoveTo's code that only a loud one gives");

impl HuntReads {
    /// What the `MoveTo`s the update makes read of `unit`, with the sprites' table `graphics`.
    pub(crate) const fn walks<'g>(
        &self,
        graphics: &'g std::collections::BTreeMap<i16, Graphic>,
        unit: &Unit,
    ) -> Walks<'g> {
        Walks {
            sprites: SpriteReads {
                defaults: self.defaults,
                graphics,
            },
            held: (unit.life.attribute_type, unit.life.amount),
            state: unit.life.state,
        }
    }
}

impl HuntAction {
    /// The quiet `MoveTo` to the object `target` at `at` within `range` (`FUN_140aa0df0` with the
    /// task's walking sprite, `+0x61` 1: it asks again when its target moves; `FUN_140a7a170`
    /// with 1), as the action list adds and starts it ([`MoveToAction::added`]): a unit within
    /// 0.01 of the target's footprint (`gap`) is there already, state 1; else state 3, or 13 for
    /// a unit without speed. None for a unit whose state is 3 or more: the list deletes it.
    ///
    /// # Errors
    ///
    /// A sprite the table lacks.
    pub(super) fn move_to_object(
        &self,
        target: i32,
        at: [f32; 3],
        range: f32,
        gap: f32,
        mover: &Mover,
        walks: &Walks<'_>,
    ) -> Result<Option<MoveToAction>, GenieError> {
        let action = MoveToAction {
            point: at,
            target,
            on_target_moved: TargetMoved::AsksAgain,
            ..MoveToAction::sub_to_point(at, range, walks.to_object(&self.task)?)
        };
        Ok(action.added(walks.state, gap, mover.stats.speed))
    }

    /// The quiet `MoveTo` to the hunt's point within the task's range (`FUN_140aa0d30` with the
    /// task's walking sprite, `+0x61` 1), as the action list adds and starts it: the point's gap
    /// is [`NO_TARGET_GAP`], so state 3, or 13 for a unit without speed. None for a unit whose
    /// state is 3 or more.
    ///
    /// # Errors
    ///
    /// A sprite the table lacks.
    pub(super) fn move_to_point(
        &self,
        mover: &Mover,
        walks: &Walks<'_>,
    ) -> Result<Option<MoveToAction>, GenieError> {
        let action = MoveToAction {
            on_target_moved: TargetMoved::AsksAgain,
            ..MoveToAction::sub_to_point(self.point, self.task.range, walks.to_point(&self.task)?)
        };
        Ok(action.added(walks.state, NO_TARGET_GAP, mover.stats.speed))
    }

    /// The Hunt's sub-action list's update (`FUN_140a992b0`: its first action's, the quiet
    /// `MoveTo`'s, [`MoveToAction::sub_update`]) and its code, which the state's case reads: 0
    /// while it walks or waits, 1 once it ended, 3 once it failed, 5 once its target was let go;
    /// 1 for an empty list ("There is no list."). The start shows the task's sprite, the motion
    /// step gives up as `gives_up` says, and a start or a move posts the units in the unit's way
    /// to its unit AI at once.
    ///
    /// # Errors
    ///
    /// The walk's and the notifies'.
    pub(super) fn update_list(
        &mut self,
        c: &mut Step<'_>,
        unit: &mut Unit,
        mut me: Me,
        gives_up: GiveUp,
    ) -> Result<u8, GenieError> {
        let Some(move_to) = &mut self.move_to else {
            return Ok(1);
        };
        me.action_state = self.state;
        let state = move_to.state;
        let held = c.w.object_state(move_to.target);
        let (mut walk, beside) = c.w.walk_beside(c.pace(), &mut unit.body, me);
        let shown = (state == MoveTo::Start).then_some(Shown {
            sprite: &mut unit.sprite,
            rng: beside.sprite_rng,
            graphic: move_to.sprite,
        });
        let mut inbox = Inbox::of(&mut unit.ai, unit.id, unit.life.state);
        let ask = PathAsk::default();
        let code = move_to.sub_update(&mut walk, ask, gives_up, held, shown, &mut inbox)?;
        if matches!(state, MoveTo::Start | MoveTo::Moving) {
            unit.post_in_the_way()?;
        }
        Ok(code)
    }
}
