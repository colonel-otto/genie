//! The `MoveTo`s the Gather's `set_state` makes (`0x140a8cd40`): what their constructors read of
//! the unit, and the two the Gather uses, one to an object and one to a point.

use crate::GenieError;
use crate::gathering::gather::{GatherAction, GatherReads};
use crate::gathering::hunt::HuntTask;
use crate::movement::Mover;
use crate::movement::move_to::{
    MoveTo, MoveToAction, MoveWait, NO_TARGET_GAP, SpriteReads, TargetMoved,
};
use crate::objects::sprite::{Graphic, sprite_id};
use crate::objects::unit::Unit;

/// What the Gather's `MoveTo` constructors read of the unit as `set_state` makes the `MoveTo`:
/// the sprites' table with the defaults of the unit's master, and what the unit carries. The
/// task's walking sprite (`FUN_140a8b660`) depends on the load.
#[derive(Clone, Copy, Debug)]
pub struct Walks<'a> {
    /// The sprites a `MoveTo` takes when the task gives none.
    pub sprites: SpriteReads<'a>,
    /// What the unit carries: its resource kind (`+0x308`) and amount (`+0x310`).
    pub held: (i16, f32),
    /// The unit's object state (`+0x640`): its action list adds a sub-action only while it is
    /// below 3 (`FUN_140a977e0`).
    pub state: u8,
}

impl Walks<'_> {
    /// The sprite of a `MoveTo` to an object (`FUN_140aa0df0`) whose maker gives the task's
    /// walking sprite ([`HuntTask::walk_sprite_id`]): the task's, else the constructor's
    /// default.
    ///
    /// # Errors
    ///
    /// A sprite the table lacks.
    pub fn to_object(&self, task: &HuntTask) -> Result<Option<Graphic>, GenieError> {
        self.sprites
            .to_object(sprite_id(task.walk_sprite_id(self.held.0, self.held.1)))
    }

    /// The sprite of a `MoveTo` to a point (`FUN_140aa0d30`), as [`Walks::to_object`].
    ///
    /// # Errors
    ///
    /// A sprite the table lacks.
    pub fn to_point(&self, task: &HuntTask) -> Result<Option<Graphic>, GenieError> {
        self.sprites
            .to_point(sprite_id(task.walk_sprite_id(self.held.0, self.held.1)))
    }
}

impl GatherReads {
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

impl GatherAction {
    /// The quiet `MoveTo` to the object `target` at `at` within the task's range
    /// (`FUN_140aa0df0` with the task's range and walking sprite, then `+0x61` 1: it asks again
    /// when its target moves; `FUN_140a7a170` with 1), as the action list adds and starts it
    /// ([`MoveToAction::added`]): a unit within 0.01 of the target's footprint
    /// (`gap`) is there already, state 1; else state 3, or 13 for a unit without speed. None for
    /// a unit whose state is 3 or more: the list deletes it.
    ///
    /// # Errors
    ///
    /// A sprite the table lacks.
    pub(super) fn move_to_object(
        &self,
        target: i32,
        at: [f32; 3],
        gap: f32,
        mover: &Mover,
        walks: &Walks<'_>,
    ) -> Result<Option<MoveToAction>, GenieError> {
        let action = MoveToAction {
            state: MoveTo::Start,
            point: at,
            target,
            range: self.task.range,
            sprite: walks.to_object(&self.task)?,
            quiet: true,
            on_target_moved: TargetMoved::AsksAgain,
            wait: MoveWait::NEW,
        };
        Ok(action.added(walks.state, gap, mover.stats.speed))
    }

    /// The quiet `MoveTo` to `point` within `range` (`FUN_140aa0d30` with the task's walking
    /// sprite, `FUN_140a7a170` with 1), whose `+0x61` is `on_target_moved` (a farm's stop sets 1,
    /// the walks of states 11, 15 and 0xf2 leave 0), as the action list adds and starts it: the
    /// point's gap is [`NO_TARGET_GAP`], so state 3, or 13 for a unit without speed. None for a
    /// unit whose state is 3 or more.
    ///
    /// # Errors
    ///
    /// A sprite the table lacks.
    pub(super) fn move_to_point(
        &self,
        point: [f32; 3],
        range: f32,
        on_target_moved: TargetMoved,
        mover: &Mover,
        walks: &Walks<'_>,
    ) -> Result<Option<MoveToAction>, GenieError> {
        let action = MoveToAction {
            on_target_moved,
            ..MoveToAction::sub_to_point(point, range, walks.to_point(&self.task)?)
        };
        Ok(action.added(walks.state, NO_TARGET_GAP, mover.stats.speed))
    }
}
