//! The Build's `MoveTo`s (`0x140aae200` cases 4, 8, 11 and 223): what their constructors read of
//! the builder, the `MoveTo`s themselves as the sub-action list adds them, and the list's update.

use crate::GenieError;
use crate::construction::build::{
    BUILDER_UNMOVING, BuildAction, BuildReads, BuildTask, LAST_TAKEN, NO_TARGET_POINT, SITE_GONE,
};
use crate::movement::move_to::{
    MoveTo, MoveToAction, MoveWait, NO_TARGET_GAP, Shown, SpriteReads, TargetMoved,
};
use crate::movement::{Me, PathAsk, footprint_gap};
use crate::objects::masters::{Master, Masters};
use crate::objects::sprite::Graphic;
use crate::objects::unit::Unit;
use crate::unit_ai::Inbox;
use crate::world::World;
use crate::world::step::Step;
use std::collections::BTreeMap;

/// What a Build's new `MoveTo` reads of its builder: the sprites its constructor takes for none
/// (the defaults of the builder's master and terrain look, and the sprites' table), the
/// builder's object state (`+0x640`: its action list adds a sub-action only below 3,
/// `FUN_140a977e0`) and its speed (slot 0x208: without one the `MoveTo` starts failed,
/// `0x140aa2100`).
#[derive(Clone, Copy, Debug)]
pub struct BuildWalks<'a> {
    /// The sprites a `MoveTo` takes when the task gives none.
    pub sprites: SpriteReads<'a>,
    /// The builder's object state.
    pub state: u8,
    /// The builder's speed.
    pub speed: f32,
}

/// An object a Build's `MoveTo` walks to: its id, its place, the builder's footprint gap to it
/// (`FUN_14118f680`) and its object state (`+0x640`), None where the world does not hold it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WalkTo {
    /// Its id.
    pub id: i32,
    /// Its place, the `MoveTo`'s point (`+0x38`).
    pub at: [f32; 3],
    /// The builder's footprint gap to it.
    pub gap: f32,
    /// Its object state.
    pub state: Option<u8>,
}

impl BuildWalks<'_> {
    /// A quiet `MoveTo` to the object `to` within `range` (`FUN_140aa0df0` with 1 and 1, then
    /// `FUN_140a7a170` with 1): its point the object's place, the sprite `given` (else the
    /// constructor's default, [`SpriteReads::to_object`]) and `on_target_moved` (`+0x61`), as
    /// the list adds it ([`MoveToAction::added`]). An object in a state above [`LAST_TAKEN`] is
    /// not its target (`FUN_140a7a230`: the id −1, the point stays) and is far
    /// ([`NO_TARGET_GAP`]).
    ///
    /// # Errors
    ///
    /// A sprite the table lacks.
    pub fn to_object(
        &self,
        to: WalkTo,
        range: f32,
        given: Option<i16>,
        on_target_moved: TargetMoved,
    ) -> Result<Option<MoveToAction>, GenieError> {
        let refused = to.state.is_some_and(|s| LAST_TAKEN < s);
        let (target, gap) = if refused {
            (-1, NO_TARGET_GAP)
        } else {
            (to.id, to.gap)
        };
        let walk = MoveToAction {
            state: MoveTo::Start,
            point: to.at,
            target,
            range,
            sprite: self.sprites.to_object(given)?,
            quiet: true,
            on_target_moved,
            wait: MoveWait::NEW,
        };
        Ok(walk.added(self.state, gap, self.speed))
    }

    /// `FUN_140aa0df0` without a target (a `set_state(4)` without a foundation): no target, its
    /// zeroed point ([`NO_TARGET_POINT`]), far ([`NO_TARGET_GAP`]); else as
    /// [`BuildWalks::to_object`].
    ///
    /// # Errors
    ///
    /// A sprite the table lacks.
    pub fn to_nothing(
        &self,
        range: f32,
        given: Option<i16>,
    ) -> Result<Option<MoveToAction>, GenieError> {
        let walk = MoveToAction {
            state: MoveTo::Start,
            point: NO_TARGET_POINT,
            target: -1,
            range,
            sprite: self.sprites.to_object(given)?,
            quiet: true,
            on_target_moved: TargetMoved::Follows,
            wait: MoveWait::NEW,
        };
        Ok(walk.added(self.state, NO_TARGET_GAP, self.speed))
    }

    /// A quiet `MoveTo` to `point` within 0 (`FUN_140aa0d30`, then `FUN_140a7a170` with 1) with
    /// the sprite `given` (else the constructor's default, [`SpriteReads::to_point`]), as the
    /// list adds it: far ([`NO_TARGET_GAP`]).
    ///
    /// # Errors
    ///
    /// A sprite the table lacks.
    pub fn to_point(
        &self,
        point: [f32; 3],
        given: Option<i16>,
    ) -> Result<Option<MoveToAction>, GenieError> {
        let walk = MoveToAction::sub_to_point(point, 0.0, self.sprites.to_point(given)?);
        Ok(walk.added(self.state, NO_TARGET_GAP, self.speed))
    }

    /// The `MoveTo` of `set_state(4)` (`0x140aae200` case 4: `FUN_140aa0df0` with the task's
    /// range `+0x30` and moving sprite `+0x40`) to the foundation `to`, or without one
    /// ([`BuildWalks::to_nothing`]).
    ///
    /// # Errors
    ///
    /// A sprite the table lacks.
    pub fn to_foundation(
        &self,
        task: &BuildTask,
        to: Option<WalkTo>,
    ) -> Result<Option<MoveToAction>, GenieError> {
        to.map_or_else(
            || self.to_nothing(task.range, task.moving),
            |to| self.to_object(to, task.range, task.moving, TargetMoved::Follows),
        )
    }
}

impl<'g> BuildWalks<'g> {
    /// What a Build's new `MoveTo` reads of `builder` of `master` (its own, or the one its task
    /// switches it to), with the sprites' table `graphics`: an object without movement has no
    /// speed (a static object's slot 0x208).
    #[must_use]
    pub fn of(builder: &Unit, master: &Master, graphics: &'g BTreeMap<i16, Graphic>) -> Self {
        Self {
            sprites: SpriteReads::of(master, builder.abilities.look.as_ref(), graphics),
            state: builder.life.state,
            speed: builder.body.mover().map_or(0.0, |m| m.stats.speed),
        }
    }
}

impl Unit {
    /// What a Build's new `MoveTo` reads of this builder ([`BuildWalks::of`] with its master),
    /// `player`'s, whose masters are `every`, with the sprites' table `graphics`.
    ///
    /// # Errors
    ///
    /// A builder whose master the table lacks.
    pub(crate) fn build_walks<'g>(
        &self,
        every: &[Masters],
        player: usize,
        graphics: &'g BTreeMap<i16, Graphic>,
    ) -> Result<BuildWalks<'g>, GenieError> {
        Ok(BuildWalks::of(self, self.master(every, player)?, graphics))
    }
}

/// The object `id` as a Build's `MoveTo` from `builder` walks to it ([`WalkTo`]: the gap
/// between their footprints with the builder's movement radii, `FUN_14118f680`), None where
/// the map does not hold it.
///
/// # Errors
///
/// A builder without movement state, and the map's lookup.
pub(super) fn walk_to(w: &World, builder: &Unit, id: i32) -> Result<Option<WalkTo>, GenieError> {
    let mover = builder.body.mover().ok_or(BUILDER_UNMOVING)?;
    let Some(t) = w.spatial.get_known(id)? else {
        return Ok(None);
    };
    let ground = [mover.stats.radius[0], mover.stats.radius[1]];
    Ok(Some(WalkTo {
        id,
        at: t.position,
        gap: footprint_gap(builder.position(), ground, t.position, t.radius),
        state: w.object_state(id),
    }))
}

impl BuildAction {
    /// The `MoveTo` of `set_state(4)` toward the Build's foundation (its place and the
    /// builder's footprint gap to it, its state as the world holds it now), or without one
    /// ([`BuildWalks::to_foundation`]).
    ///
    /// # Errors
    ///
    /// A builder without movement state or its master, a foundation without a place, a sprite
    /// the table lacks.
    pub(super) fn walk_to_foundation(
        &self,
        c: &Step<'_>,
        builder: &Unit,
    ) -> Result<(Option<WalkTo>, Option<MoveToAction>), GenieError> {
        let walks = builder.build_walks(&c.w.masters, c.player, &c.w.graphics)?;
        let to = if self.target == -1 {
            None
        } else {
            Some(walk_to(c.w, builder, self.target)?.ok_or(SITE_GONE)?)
        };
        Ok((to, walks.to_foundation(&self.task, to)?))
    }

    /// The sub-action list's update (`FUN_140a992b0`: its first action's, the quiet `MoveTo`'s,
    /// [`MoveToAction::sub_update`]) and its code: 0 while it walks or waits, 1 once it ended,
    /// 3 once it failed, 5 once its target was let go; 1 for an empty list ("There is no
    /// list."). Its start shows its sprite; the motion step gives up as [`BuildAction::gives_up`]
    /// says.
    ///
    /// # Errors
    ///
    /// The walk's and the notifies'.
    pub(super) fn step_walk(
        &mut self,
        c: &mut Step<'_>,
        builder: &mut Unit,
        me: Me,
        reads: &BuildReads,
    ) -> Result<u8, GenieError> {
        let gives_up = Self::gives_up(c, builder, reads);
        let Some(move_to) = &mut self.move_to else {
            return Ok(1);
        };
        let held = c.w.object_state(move_to.target);
        let (mut walk, beside) = c.w.walk_beside(c.pace(), &mut builder.body, me);
        let shown = (move_to.state == MoveTo::Start).then_some(Shown {
            sprite: &mut builder.sprite,
            rng: beside.sprite_rng,
            graphic: move_to.sprite,
        });
        let mut inbox = Inbox::of(&mut builder.ai, builder.id, builder.life.state);
        let ask = PathAsk::default();
        move_to.sub_update(&mut walk, ask, gives_up, held, shown, &mut inbox)
    }
}
