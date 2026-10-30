//! The `MoveTo` as a unit's own top action ([`MoveToAction`]): its constructors, its start and
//! its update.

use crate::GenieError;
use crate::combat::abilities::Look;
use crate::map::spatial::Spatial;
use crate::math::rng::Well512;
use crate::movement::move_to::Shown;
use crate::movement::move_to::{
    Follow, Goal, MoveTo, MoveWait, Walk, follow_path_end, start_walk_shown, target_moved,
    walk_or_give_up,
};
use crate::movement::{Mover, PathAsk, footprint_gap};
use crate::objects::masters::Master;
use crate::objects::masters::ids::TOWN_CENTRE;
use crate::objects::sprite::{ActiveSprite, Graphic, new_sprite, sprite_id, standing, walking};
use genie_diff::Diff;

/// A `MoveTo` (`RGE_Action_Move_To`): a unit's own top action (a wandering animal's, a moved or
/// following unit's; its `+0x60` and `+0x61` are 0: it is not quiet), or the sub-action of an
/// action that walks ([`MoveToAction::sub_to_point`]).
#[derive(Clone, Copy, Debug, PartialEq, Diff)]
pub struct MoveToAction {
    /// Its state (`+0x18`).
    pub state: MoveTo,
    /// The point it walks to (`+0x38`..`+0x40`), when it has no target object.
    pub goal: Option<[f32; 3]>,
    /// The target object it walks to (`+0x20`, its id at `+0x30`), −1 for a point; its point is
    /// the mover's [`Mover::move_to_point`].
    pub target: i32,
    /// `+0x68`.
    pub range: f32,
    /// The sprite it shows when it turns to state 4 (`+0x58`, the save's second `short_val`):
    /// the one its maker gave, else its constructor's default ([`point_sprite`],
    /// [`object_sprite`]); None for none (null).
    pub sprite: Option<Graphic>,
    /// `+0x60`: quiet. An action makes its sub-action `MoveTo` quiet (`FUN_140a7a170` with 1):
    /// its `set_state` 1, 13 and 14 show no standing sprite (`0x140aa29e0`), and its update
    /// posts no notify 0x1fa, 0x1f9 or 0x1fb (`0x140aa2a70`). A `MoveTo` of its own is not quiet.
    /// The save does not hold it: the import takes its maker's.
    pub quiet: bool,
    /// Its wait after the motion step gave up.
    pub wait: MoveWait,
}

/// A `MoveTo` without a target object or a point.
const GOAL_LACKING: GenieError = GenieError::unsupported("a MoveTo of its own without a goal");

/// A `MoveTo`'s start counts the unit at its target within this gap (`0x140aa2100`,
/// `DAT_142eb99cc`).
const ARRIVED: f32 = crate::math::image::move_to::ARRIVED;
/// A `MoveTo` to an object the world does not hold.
pub(crate) const TARGET_LACKING: GenieError =
    GenieError::unsupported("a MoveTo to an object the world lacks");
/// Livestock's `MoveTo` to a town centre walks to its place less this on both axes
/// (`FUN_140aa0df0`, `DAT_142eae8d0`).
const TOWN_CENTRE_OFFSET: f32 = crate::math::image::artifact::TOWN_CENTRE_OFFSET;

/// `FUN_140aa0df0`'s test of a `MoveTo`'s target object of master `master` in `state`:
/// livestock (`livestock`, the unit's class 58) moving to a town centre (master 109) holds none,
/// and no `MoveTo` holds a target in state 7 or later (`FUN_140a7a230`); any other target is
/// held, and a `MoveTo` of its own (not quiet) takes it (slot 0x178).
#[must_use]
pub const fn holds_object(livestock: bool, master: i16, state: u8) -> bool {
    // The town centre's master (`FUN_140aa0df0`'s `0x6109` raw), to which livestock walks beside.
    !(livestock && master == TOWN_CENTRE) && state < 7
}

/// What one update of a [`MoveToAction`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MoveStep {
    /// It is done (its update in state 1 returns 1): the action list removes it.
    pub done: bool,
    /// Its target left its point: it told the unit (notify 0x1fc) and turned to state 3.
    pub target_moved: bool,
    /// The motion step could not go on (its result 1): the caller sets state 13, or for a
    /// human's villager the wait of state 2 (`LAB_140aa320c`).
    pub gave_up: bool,
    /// It had failed (state 13): its update returns 3 after the notify 0x1f9 (an own `MoveTo` is
    /// never quiet); the action list removes it.
    pub failed: bool,
}

/// The range a `MoveTo` to an object (`FUN_140aa0df0`) takes for `range`: one made with none
/// (0.0; `UCOMISS`, so −0.0 too and never a NaN) takes the unit's slot 0x268. A building's
/// (`FUN_1411c5d80`, master type 80) is `image::order::CART_MOVE_RANGE` (0.75) when its slot
/// 0x4f0 holds (`FUN_1411d4980` -> `FUN_1415c60b0`: its master has a task of action type 0x99, a
/// mule cart's); any other's (`FUN_14106a6d0`) is 0. A `MoveTo` to a point (`FUN_140aa0d30`)
/// keeps its range.
#[must_use]
pub fn object_range(range: f32, master: Option<&Master>) -> f32 {
    if range != 0.0 {
        return range;
    }
    let cart = master.is_some_and(|m| {
        m.kind == crate::objects::masters::master_type::BUILDING
            && m.tasks
                .iter()
                .any(|t| t.action_type == crate::construction::build::CART_TASK)
    });
    if cart {
        crate::math::image::order::CART_MOVE_RANGE
    } else {
        0.0
    }
}

/// The sprite (`+0x58`) of a `MoveTo` to a point whose maker gives none (`FUN_140aa0d30` with
/// 0): slot 0x7c8's, the walking sprite ([`walking`]), else slot 0x798's, the standing one
/// ([`standing`]); `look` and `goods` as those read them.
#[must_use]
pub fn point_sprite(master: &Master, look: Option<&Look>, goods: bool) -> Option<i16> {
    walking(master, look, goods).or_else(|| standing(master, look, goods))
}

/// The sprite (`+0x58`) of a `MoveTo` to an object whose maker gives none (`FUN_140aa0df0` with
/// 0): the master's walking sprite (`+0x748`), else its standing one (`+0x470`); no terrain look.
#[must_use]
pub fn object_sprite(master: &Master) -> Option<i16> {
    sprite_id(master.move_sprite).or_else(|| sprite_id(master.standing_sprite))
}

/// The sprite the unit AI's moves give their `MoveTo` (slot 0x340, `FUN_1410866e0`, with
/// `walk` its seventh argument; slot 0x348, `FUN_141086440`, never walks): the master's running
/// sprite (`+0x750`) unless the move walks, else (or without one) its walking sprite (`+0x748`);
/// no terrain look. None leaves the constructor's default ([`point_sprite`], [`object_sprite`]).
#[must_use]
pub fn ai_sprite(master: &Master, walk: bool) -> Option<i16> {
    let run = sprite_id(master.run_sprite).filter(|_| !walk);
    run.or_else(|| sprite_id(master.move_sprite))
}

/// What a `MoveTo`'s constructors take for the sprite (`+0x58`) when their maker gives none,
/// read from the unit's master and terrain look as the `MoveTo` is made: `FUN_140aa0d30`'s
/// ([`point_sprite`]) and `FUN_140aa0df0`'s ([`object_sprite`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MoveDefaults {
    /// `FUN_140aa0d30`'s, a `MoveTo` to a point.
    pub point: Option<i16>,
    /// `FUN_140aa0df0`'s, a `MoveTo` to an object.
    pub object: Option<i16>,
}

impl MoveDefaults {
    /// The defaults of a unit of `master` with the terrain look `look` (a trade cart's goods are
    /// not held: none).
    #[must_use]
    pub fn of(master: &Master, look: Option<&Look>) -> Self {
        Self {
            point: point_sprite(master, look, false),
            object: object_sprite(master),
        }
    }

    /// The sprite of a `MoveTo` to a point (`FUN_140aa0d30`) whose maker gives `given`.
    #[must_use]
    pub const fn to_point(self, given: Option<i16>) -> Option<i16> {
        match given {
            Some(id) => Some(id),
            None => self.point,
        }
    }

    /// The sprite of a `MoveTo` to an object (`FUN_140aa0df0`) whose maker gives `given`.
    #[must_use]
    pub const fn to_object(self, given: Option<i16>) -> Option<i16> {
        match given {
            Some(id) => Some(id),
            None => self.object,
        }
    }
}

/// The unit's sprite as its own `MoveTo`'s `set_state` (`0x140aa29e0`) shows it with slot 0x68
/// ([`new_sprite`]): state 4 the `MoveTo`'s sprite ([`MoveToAction::sprite`]); states 1 and 13
/// (an own `MoveTo` is not quiet) slot 0x798's ([`standing`]). Slot 0x68 shows the master's
/// standing sprite (`+0x470`) for a null one.
#[derive(Debug)]
pub struct Shows<'a> {
    /// The unit's sprite.
    pub sprite: &'a mut Option<ActiveSprite>,
    /// The Sprite stream.
    pub rng: &'a mut Well512,
    /// Slot 0x798's sprite, the unit's standing one.
    pub standing: Option<Graphic>,
    /// The master's standing sprite (`+0x470`), which slot 0x68 shows for a null one.
    pub master_standing: Option<Graphic>,
}

impl MoveToAction {
    /// A new `MoveTo` of its own to the object `target` within `range` (`FUN_140aa0df0`; its point
    /// `+0x38` the target's place, [`Mover::move_to_point`], which a path request renews), with
    /// its `sprite` ([`MoveToAction::sprite`]), as its start (slot 0x808 ->
    /// `0x140aa2100`) leaves it: a unit more than [`ARRIVED`] from the target's footprint
    /// (`FUN_14118f680`) in state 3 with speed (slot 0x208), 13 without; a unit at the target
    /// already in state 1, which its next update ends. States 1 and 13 (`0x140aa29e0`, not quiet)
    /// show the master's standing sprite ([`Mover::shows_standing`]; the sprite itself and
    /// `+0x7b8`, cleared, are the caller's, [`MoveToAction::shows_standing`]). `me` is the unit's
    /// place.
    pub fn to_target(
        world: &Spatial,
        me: [f32; 3],
        mover: &mut Mover,
        target: i32,
        range: f32,
        sprite: Option<Graphic>,
    ) -> Result<Self, GenieError> {
        let t = world.get_known(target)?.ok_or(TARGET_LACKING)?;
        let ground = [mover.stats.radius[0], mover.stats.radius[1]];
        mover.move_to_point = Some(t.position);
        let state = if footprint_gap(me, ground, t.position, t.radius) <= ARRIVED {
            MoveTo::Arrived
        } else if mover.stats.speed == 0.0 {
            MoveTo::Failed
        } else {
            MoveTo::Start
        };
        if state != MoveTo::Start {
            mover.shows_standing();
        }
        Ok(Self {
            state,
            goal: None,
            target,
            range,
            sprite,
            quiet: false,
            wait: MoveWait::default(),
        })
    }

    /// A new `MoveTo` of its own (quiet, `+0x60`, 0) to the object `target` within `range`
    /// (`FUN_140aa0df0`), then its start (slot 0x808 -> `0x140aa2100`). Livestock (`livestock`,
    /// the unit's class 58) moving to a town centre (master 109) holds no target and walks to
    /// the centre's place less [`TOWN_CENTRE_OFFSET`] on both axes; a target in a `state` of 7
    /// or later is not held either (`FUN_140a7a230`: its id stays −1) and the `MoveTo` walks to
    /// its place, as a `MoveTo` to that point starts ([`MoveToAction::to_point`]). Any other
    /// target is held ([`MoveToAction::to_target`]), and the caller takes it (slot
    /// 0x178, [`MoveToAction::target`] not −1).
    ///
    /// # Errors
    ///
    /// A target the world lacks.
    pub fn to_object(
        world: &Spatial,
        me: [f32; 3],
        mover: &mut Mover,
        (target, state): (i32, u8),
        range: f32,
        (sprite, livestock): (Option<Graphic>, bool),
    ) -> Result<Self, GenieError> {
        let t = world.get_known(target)?.ok_or(TARGET_LACKING)?;
        let at = t.position;
        let goal = if livestock && t.master == TOWN_CENTRE {
            [
                at[0] - TOWN_CENTRE_OFFSET,
                at[1] - TOWN_CENTRE_OFFSET,
                at[2],
            ]
        } else if holds_object(livestock, t.master, state) {
            return Self::to_target(world, me, mover, target, range, sprite);
        } else {
            at
        };
        Ok(Self::to_point(goal, range, sprite, Some(mover)))
    }

    /// A new `MoveTo` of its own (not quiet) to `point` within `range` (`FUN_140aa0d30`), with its
    /// `sprite` ([`MoveToAction::sprite`]), as its start (slot 0x808 ->
    /// `0x140aa2100`) leaves it. Without a target object the start's gap (`FUN_14118f680` with
    /// none) is `f32::MAX`, never within 0.01: a unit with speed (slot 0x208, `UCOMISS`: a NaN
    /// speed has some) is in state 3, one without in 13 (`0x140aa29e0`, not quiet: the master's
    /// standing sprite, [`Mover::shows_standing`]; the sprite itself and `+0x7b8`, cleared, are the
    /// caller's, [`MoveToAction::shows_standing`]). A unit without movement state has no speed.
    #[must_use]
    pub fn to_point(
        point: [f32; 3],
        range: f32,
        sprite: Option<Graphic>,
        mover: Option<&mut Mover>,
    ) -> Self {
        let state = match mover {
            Some(m) if m.stats.speed != 0.0 => MoveTo::Start,
            Some(m) => {
                m.shows_standing();
                MoveTo::Failed
            }
            None => MoveTo::Failed,
        };
        Self {
            state,
            goal: Some(point),
            target: -1,
            range,
            sprite,
            quiet: false,
            wait: MoveWait::default(),
        }
    }

    /// Where it walks (`+0x30`, else `+0x38`..`+0x40`): its target object, else its point.
    ///
    /// # Errors
    ///
    /// A `MoveTo` with neither.
    pub const fn walks_to(&self) -> Result<Goal, GenieError> {
        match (self.target, self.goal) {
            (-1, Some(point)) => Ok(Goal::Point(point)),
            (-1, None) => Err(GOAL_LACKING),
            (id, _) => Ok(Goal::Target(id)),
        }
    }

    /// It ended at its start, the unit at its target (state 1).
    #[must_use]
    pub fn arrived(&self) -> bool {
        self.state == MoveTo::Arrived
    }

    /// Its start ended it (state 1 at its target, or 13 for a unit without speed), whose
    /// `set_state` (`0x140aa29e0`, not quiet) shows the master's standing sprite and clears the
    /// unit's `+0x7b8`: the caller's ([`crate::objects::unit::Unit::shows_standing_sprite`]).
    #[must_use]
    pub const fn shows_standing(&self) -> bool {
        matches!(self.state, MoveTo::Arrived | MoveTo::Failed)
    }

    /// The point it walks to (`+0x38`..`+0x40`): its own, or for a target object the mover's
    /// [`Mover::move_to_point`] (the target's place when it was made or last asked for a path).
    #[must_use]
    pub fn point(&self, mover: Option<&Mover>) -> Option<[f32; 3]> {
        self.goal.or_else(|| mover.and_then(|m| m.move_to_point))
    }

    /// A `MoveTo` of its own is not quiet: its `set_state` 1 or 13 shows slot 0x798's sprite
    /// ([`Shows::standing`], slot 0x68 with it, [`new_sprite`]) and clears the unit's `+0x7b8`.
    fn ended(&self, walk: &mut Walk<'_>, shows: &mut Shows<'_>) {
        if matches!(self.state, MoveTo::Arrived | MoveTo::Failed) {
            if let Some(g) = shows.standing.or(shows.master_standing) {
                new_sprite(shows.sprite, walk.body.move_stats_mut(), g, shows.rng);
            }
            walk.set_anim_speed(0.0);
        }
    }

    /// One update (`0x140aa2240`) of the unit whose sprite `shows` holds, its state 3 with what
    /// the unit's AI says (`ask`, [`crate::movement::move_to::start_walk_shown`]): a walk goes on ([`MoveToAction::update_moving`]), a start asks for
    /// its path ([`MoveToAction::update_start`]), a wait runs by dt and at its end the `MoveTo`
    /// asks again (`set_state` 3). A target that is gone (the update's first check: notify
    /// 0x1fb, code 4) is not modelled.
    pub fn update(
        &mut self,
        mut walk: Walk<'_>,
        ask: PathAsk,
        mut shows: Shows<'_>,
    ) -> Result<MoveStep, GenieError> {
        let mut step = MoveStep::default();
        if self.target != -1 && walk.world.get_known(self.target)?.is_none() {
            return Err(GenieError::unsupported(
                "a MoveTo of its own whose target is gone",
            ));
        }
        match self.state {
            MoveTo::Moving => self.update_moving(&mut walk, &mut step, &mut shows)?,
            MoveTo::Start => self.update_start(&mut walk, ask, &mut step, &mut shows)?,
            MoveTo::Arrived => step.done = true,
            // case 2: the timer runs by dt; at its end the MoveTo asks again (set_state 3)
            MoveTo::Waiting => {
                self.wait.timer += walk.dt;
                if self.wait.delay <= self.wait.timer {
                    self.wait.timer = 0.0;
                    self.state = MoveTo::Start;
                }
            }
            // 0x140aa2a70's state 13: the notify (the caller's), then code 3
            MoveTo::Failed => step.failed = true,
        }
        Ok(step)
    }

    /// The update's walk (state 4): a target that left its point makes the `MoveTo` follow it
    /// (the top path's end, [`follow_path_end`]) and turn to state 3 before the motion step
    /// (`LAB_140aa320c`), which moving on leaves it in state 3; else the motion step alone. A
    /// motion step that gives up leaves the wait or the failure to the caller.
    fn update_moving(
        &mut self,
        walk: &mut Walk<'_>,
        step: &mut MoveStep,
        shows: &mut Shows<'_>,
    ) -> Result<(), GenieError> {
        let follow = if self.target == -1 {
            Follow::Stays
        } else {
            target_moved(walk, self.target, false)?
        };
        if let Follow::PathEnd(p) = follow {
            follow_path_end(walk, p)?;
        }
        if follow.again() {
            step.target_moved = true;
            match walk_or_give_up(walk.reborrow())? {
                Some(MoveTo::Moving) => self.state = MoveTo::Start,
                Some(other) => self.state = other,
                // the motion step gave up: the wait or the failure, as below
                None => {
                    self.state = MoveTo::Start;
                    step.gave_up = true;
                }
            }
        } else {
            match walk_or_give_up(walk.reborrow())? {
                Some(state) => self.state = state,
                None => step.gave_up = true,
            }
        }
        self.ended(walk, shows);
        Ok(())
    }

    /// The update's start (state 3): after a wait `+0x6c` goes from 1 to 2 as it asks again; the
    /// goal is its point or its target; it asks for its path and moves (state 4), whose
    /// `set_state` shows its sprite ([`MoveToAction::sprite`], [`start_walk_shown`]) before the
    /// motion step.
    fn update_start(
        &mut self,
        walk: &mut Walk<'_>,
        ask: PathAsk,
        step: &mut MoveStep,
        shows: &mut Shows<'_>,
    ) -> Result<(), GenieError> {
        if self.wait.state == 1 {
            self.wait.state = 2;
        }
        let goal = self.walks_to()?;
        self.state = MoveTo::Moving;
        let shown = Shown {
            sprite: &mut *shows.sprite,
            rng: &mut *shows.rng,
            graphic: self.sprite.or(shows.master_standing),
        };
        match start_walk_shown(
            walk.reborrow(),
            (goal, self.range, self.quiet),
            ask,
            Some(shown),
        )? {
            Some(state) => self.state = state,
            None => step.gave_up = true,
        }
        self.ended(walk, shows);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::spatial::Placed;
    use crate::movement::move_to::tests::sheep;

    /// A sprite `id` whose speed factor is `factor`, looped.
    const fn graphic(id: i16, factor: f32) -> Graphic {
        Graphic {
            id,
            frames: 10,
            facets: 16,
            speed_multiplier: factor,
            frame_duration: 0.1,
            replay_delay: 0.0,
            flags: 1,
        }
    }

    /// The running sprite (factor 1.5), the walking one (1) and the standing one (1.25).
    const RUNNING: Graphic = graphic(13154, 1.5);
    const WALKING: Graphic = graphic(13153, 1.0);
    const STANDING: Graphic = graphic(13151, 1.25);

    /// A `MoveTo` of its own to the house 10028's point, in `state`, with the sprite `sprite`.
    fn own_move_to(state: MoveTo, sprite: Option<Graphic>, target: i32) -> MoveToAction {
        MoveToAction {
            sprite,
            state,
            goal: (target == -1).then_some([30.0, 88.0, 0.0]),
            target,
            range: 0.1,
            quiet: false,
            wait: MoveWait {
                state: 1,
                timer: 0.0,
                delay: 0.15,
            },
        }
    }

    /// The unit's sprite and the Sprite stream, which a `MoveTo` of its own shows on.
    #[derive(Default)]
    struct Looks {
        sprite: Option<ActiveSprite>,
        rng: Well512,
    }

    impl Looks {
        /// The view of [`Shows`], with [`STANDING`] as slot 0x798's and the master's.
        fn shows(&mut self) -> Shows<'_> {
            Shows {
                sprite: &mut self.sprite,
                rng: &mut self.rng,
                standing: Some(STANDING),
                master_standing: Some(STANDING),
            }
        }

        /// The id of the sprite the unit shows.
        fn shown(&self) -> Option<i16> {
            self.sprite.map(|s| s.graphic.id)
        }
    }

    /// A `MoveTo` of its own (`0x140aa2240`) with [`WalkRig`]'s walker (its master's speed
    /// 0.8): a start counts a wait's ask (`+0x6c` 1 to 2), asks for its path and turns to state
    /// 4, whose `set_state` (`0x140aa29e0`) shows its sprite (the running one: top speed 0.8 ×
    /// 1.5) before the motion step; a move runs the motion step and keeps moving; one at its
    /// target is done, a failed one reports it; a wait runs by the walk's dt and asks again at
    /// its delay. A start without a goal and a target that is gone are not modelled.
    ///
    /// [`WalkRig`]: crate::movement::tests::WalkRig
    #[test]
    fn a_move_to_of_its_own_starts_moves_and_reports_its_end() {
        use crate::movement::tests::WalkRig;
        let mut rig = WalkRig::on_the_path();
        let mut looks = Looks::default();
        let mut m = own_move_to(MoveTo::Start, Some(RUNNING), -1);
        let step = m
            .update(rig.walk(0), PathAsk::default(), looks.shows())
            .unwrap();
        assert_eq!(step, MoveStep::default());
        assert_eq!((m.state, m.wait.state), (MoveTo::Moving, 2));
        assert_eq!(looks.shown(), Some(13154));
        assert_eq!(rig.mover_mut().stats.speed, 0.8 * 1.5);
        let step = m
            .update(rig.walk(0), PathAsk::default(), looks.shows())
            .unwrap();
        assert_eq!((m.state, step), (MoveTo::Moving, MoveStep::default()));
        let mut on = own_move_to(MoveTo::Start, Some(WALKING), 10028);
        let _ = on
            .update(rig.walk(0), PathAsk::default(), looks.shows())
            .unwrap();
        assert_eq!(
            (on.state, looks.shown(), rig.mover_mut().stats.speed),
            (MoveTo::Moving, Some(13153), 0.8)
        );
        let ended = |state| {
            let mut rig = WalkRig::on_the_path();
            let mut m = own_move_to(state, Some(WALKING), -1);
            let step = m
                .update(rig.walk(0), PathAsk::default(), Looks::default().shows())
                .unwrap();
            (step, m.state)
        };
        let done = MoveStep {
            done: true,
            ..MoveStep::default()
        };
        let failed = MoveStep {
            failed: true,
            ..MoveStep::default()
        };
        assert_eq!(ended(MoveTo::Arrived), (done, MoveTo::Arrived));
        assert_eq!(ended(MoveTo::Failed), (failed, MoveTo::Failed));
        // the walk is a tenth of a second: the wait of 0.15 asks again on its second update
        let mut w = own_move_to(MoveTo::Waiting, Some(WALKING), -1);
        let _ = w
            .update(rig.walk(0), PathAsk::default(), looks.shows())
            .unwrap();
        assert_eq!((w.state, w.wait.timer), (MoveTo::Waiting, 0.1));
        let _ = w
            .update(rig.walk(0), PathAsk::default(), looks.shows())
            .unwrap();
        assert_eq!((w.state, w.wait.timer), (MoveTo::Start, 0.0));
        let reason = |mut m: MoveToAction| {
            let mut rig = WalkRig::on_the_path();
            m.update(rig.walk(0), PathAsk::default(), Looks::default().shows())
                .unwrap_err()
                .reason()
        };
        let no_goal = MoveToAction {
            goal: None,
            ..own_move_to(MoveTo::Start, Some(WALKING), -1)
        };
        assert_eq!(reason(no_goal), "a MoveTo of its own without a goal");
        assert_eq!(
            reason(own_move_to(MoveTo::Moving, Some(WALKING), 77)),
            "a MoveTo of its own whose target is gone"
        );
    }

    /// The `set_state` of a `MoveTo` of its own (`0x140aa29e0`, not quiet): turning to state 4
    /// shows its sprite and a null one the master's standing sprite (slot 0x68 with none); the
    /// end in range at its start (state 1) shows slot 0x798's sprite and clears `+0x7b8`, and so
    /// does a walk that arrives; the top speed follows each sprite (the master's 0.8 times its
    /// factor).
    #[test]
    fn a_move_to_of_its_own_shows_its_sprite_as_it_moves_and_the_standing_one_at_its_end() {
        use crate::movement::tests::WalkRig;
        let started = |sprite: Option<Graphic>| {
            let mut rig = WalkRig::on_the_path();
            let mut looks = Looks::default();
            let mut m = own_move_to(MoveTo::Start, sprite, -1);
            let _ = m
                .update(rig.walk(0), PathAsk::default(), looks.shows())
                .unwrap();
            (m.state, looks.shown(), rig.mover_mut().stats.speed)
        };
        assert_eq!(
            started(Some(RUNNING)),
            (MoveTo::Moving, Some(13154), 0.8 * 1.5)
        );
        assert_eq!(started(None), (MoveTo::Moving, Some(13151), 0.8 * 1.25));
        // at the point already: state 1, which shows the standing sprite
        let mut rig = WalkRig::on_the_path();
        rig.body.set_anim_speed(1.0);
        let mut looks = Looks::default();
        let mut at = own_move_to(MoveTo::Start, Some(RUNNING), -1);
        at.goal = Some([28.0, 89.0, 0.0]);
        let ask = PathAsk {
            ends_in_range: true,
            ..PathAsk::default()
        };
        let _ = at.update(rig.walk(0), ask, looks.shows()).unwrap();
        assert_eq!(at.state, MoveTo::Arrived);
        assert_eq!(looks.shown(), Some(13151));
        assert_eq!(rig.mover_mut().stats.speed, 0.8 * 1.25);
        assert_eq!(rig.mover_mut().animated.speed, 0.0);
    }

    /// The sprite a `MoveTo` takes when its maker gives none: `FUN_140aa0d30`'s slot 0x7c8 (a
    /// terrain look's walking sprite first), else slot 0x798; `FUN_140aa0df0`'s master's walking
    /// sprite, else its standing one, never a look's. The unit AI's moves give the running sprite
    /// unless they walk, else the walking one, and none without either.
    #[test]
    fn a_move_to_without_a_sprite_takes_its_constructor_s() {
        let master = Master {
            run_sprite: 13154,
            move_sprite: 13153,
            standing_sprite: 13151,
            ..Master::default()
        };
        assert_eq!(point_sprite(&master, None, false), Some(13153));
        assert_eq!(object_sprite(&master), Some(13153));
        assert_eq!(ai_sprite(&master, false), Some(13154));
        assert_eq!(ai_sprite(&master, true), Some(13153));
        let look = Look {
            standing: Some(501),
            walking: Some(500),
            attacking: None,
            running: None,
            dying: None,
            undead: None,
        };
        assert_eq!(point_sprite(&master, Some(&look), false), Some(500));
        let still = Master {
            run_sprite: -1,
            move_sprite: -1,
            ..master
        };
        assert_eq!(point_sprite(&still, None, false), Some(13151));
        assert_eq!(object_sprite(&still), Some(13151));
        assert_eq!(ai_sprite(&still, false), None);
        let standing_look = Look {
            walking: None,
            ..look
        };
        assert_eq!(point_sprite(&still, Some(&standing_look), false), Some(501));
    }

    /// The walk of a `MoveTo` of its own on the house 10028 (`0x140aa2240`, state 4): a target
    /// that stays walks on; one that left the `MoveTo`'s point (by more than the tolerance) has
    /// a walker on its path's last stretch follow it there, and any other tell the unit and turn
    /// the `MoveTo` to state 3 before the motion step.
    #[test]
    fn a_move_to_of_its_own_turns_to_its_start_when_its_target_moved() {
        use crate::movement::tests::WalkRig;
        let walked = |point: [f32; 3], waypoint| {
            let mut rig = WalkRig::on_the_path();
            rig.mover_mut().move_to_point = Some(point);
            rig.mover_mut().paths[0].waypoint = waypoint;
            let mut m = own_move_to(MoveTo::Moving, Some(WALKING), 10028);
            let step = m
                .update(rig.walk(0), PathAsk::default(), Looks::default().shows())
                .unwrap();
            (m.state, step.target_moved)
        };
        assert_eq!(walked([30.0, 88.0, 0.0], 1), (MoveTo::Moving, false));
        assert_eq!(walked([36.0, 88.0, 0.0], 1), (MoveTo::Moving, false));
        assert_eq!(walked([36.0, 88.0, 0.0], 0), (MoveTo::Start, true));
    }

    /// `FUN_140aa0df0` with no range takes the unit's slot 0x268: 0.75 for a building whose
    /// master has a task of action type 0x99 (431096717's mule carts of player 2 at update 1),
    /// 0 for a building without one, for a unit with one, and without a master; a range given
    /// stays.
    #[test]
    fn a_move_to_an_object_without_a_range_takes_a_mule_cart_s() {
        use crate::objects::masters::{Master, Task, master_type::BUILDING};
        let cart_task = Task {
            action_type: crate::construction::build::CART_TASK,
            ..Task::default()
        };
        let cart = Master {
            kind: BUILDING,
            tasks: vec![Task::default(), cart_task],
            ..Master::default()
        };
        let building = Master {
            kind: BUILDING,
            tasks: vec![Task::default()],
            ..Master::default()
        };
        let unit = Master {
            kind: 70,
            ..cart.clone()
        };
        assert_eq!(object_range(0.0, Some(&cart)), 0.75);
        assert_eq!(object_range(-0.0, Some(&cart)), 0.75);
        assert_eq!(object_range(0.0, Some(&building)), 0.0);
        assert_eq!(object_range(0.0, Some(&unit)), 0.0);
        assert_eq!(object_range(0.0, None), 0.0);
        assert_eq!(object_range(0.5, Some(&cart)), 0.5);
        assert!(object_range(f32::NAN, Some(&cart)).is_nan());
    }

    #[test]
    fn a_move_to_an_object_at_hand_starts_arrived_and_shows_the_standing_sprite() {
        let mut world = Spatial::new([120, 120]);
        world.insert(9185, sheep(30.4, 40.0));
        world.insert(9186, sheep(35.0, 40.0));
        let mut walker = crate::gathering::hunt::tests::walker();
        walker.stats.stand_speed = 0.5;
        // footprints 0 apart: within ARRIVED, state 1 with the standing sprite's speed
        let me = [30.0, 40.0, 0.0];
        let at_hand = MoveToAction::to_target(&world, me, &mut walker, 9185, 0.0, None).unwrap();
        assert!(at_hand.arrived());
        assert_eq!(walker.stats.speed, 0.5);
        // 4.6 tiles on it starts walking (state 3)
        let mut walker = crate::gathering::hunt::tests::walker();
        let far = MoveToAction::to_target(&world, me, &mut walker, 9186, 0.0, None).unwrap();
        assert!(!far.arrived());
        assert_eq!(walker.stats.speed, 0.8);
    }

    /// `FUN_140aa0df0` on an object: a live target is held (the caller takes it); a target in
    /// state 7 is not (`FUN_140a7a230`) and the `MoveTo` walks to its place; livestock moving to
    /// a town centre walks to its place less 0.2 on both axes without a target. Without a target
    /// the start is state 3, or 13 for a unit without speed.
    #[test]
    fn a_move_to_an_object_holds_it_unless_it_is_gone_or_a_town_centre_for_livestock() {
        let mut world = Spatial::new([120, 120]);
        world.insert(9186, sheep(35.0, 40.0));
        world.insert(
            9149,
            Placed {
                master: 109,
                radius: [2.0, 2.0],
                ..sheep(50.0, 40.0)
            },
        );
        let me = [30.0, 40.0, 0.0];
        let to = |target, livestock, speed: f32| {
            let mut walker = crate::gathering::hunt::tests::walker();
            walker.stats.speed = speed;
            MoveToAction::to_object(&world, me, &mut walker, target, 0.0, (None, livestock))
                .unwrap()
        };
        let held = to((9186, 2), false, 0.8);
        assert_eq!(
            (held.target, held.goal, held.state),
            (9186, None, MoveTo::Start)
        );
        let gone = to((9186, 7), false, 0.8);
        assert_eq!(
            (gone.target, gone.goal, gone.state),
            (-1, Some([35.0, 40.0, 0.0]), MoveTo::Start)
        );
        assert_eq!(to((9186, 7), false, 0.0).state, MoveTo::Failed);
        // a villager to the town centre holds it; livestock walks beside it
        assert_eq!(to((9149, 2), false, 0.8).target, 9149);
        let beside = to((9149, 2), true, 0.8);
        assert_eq!(
            (beside.target, beside.goal),
            (-1, Some([50.0 - 0.2, 40.0 - 0.2, 0.0]))
        );
    }

    /// `FUN_140aa0d30` and its start `0x140aa2100`: a point is never within 0.01 (no target, so
    /// `f32::MAX`): state 3 for a unit with speed (a NaN speed has some), 13 for one without,
    /// whose not quiet `set_state` shows the standing sprite (its speed); without movement state
    /// a unit has no speed. Its point is `+0x38`.
    #[test]
    fn a_move_to_a_point_starts_unless_its_unit_has_no_speed() {
        let point = [12.0, 34.0, 1.0];
        let start = |speed: f32| {
            let mut walker = crate::gathering::hunt::tests::walker();
            walker.stats.speed = speed;
            walker.stats.stand_speed = 0.25;
            let m = MoveToAction::to_point(point, 0.5, Some(RUNNING), Some(&mut walker));
            (m, walker.stats.speed)
        };
        let (moving, speed) = start(0.8);
        assert_eq!(
            (
                moving.state,
                moving.goal,
                moving.target,
                moving.range,
                moving.sprite
            ),
            (MoveTo::Start, Some(point), -1, 0.5, Some(RUNNING))
        );
        assert_eq!(speed, 0.8);
        assert!(!moving.shows_standing());
        assert_eq!(moving.point(None), Some(point));
        assert_eq!(start(f32::NAN).0.state, MoveTo::Start);
        let (stuck, speed) = start(0.0);
        assert_eq!(stuck.state, MoveTo::Failed);
        assert!(stuck.shows_standing());
        assert_eq!(speed, 0.25);
        let none = MoveToAction::to_point(point, 0.0, None, None);
        assert_eq!(none.state, MoveTo::Failed);
    }

    /// `FUN_140aa0df0` puts the target's place in `+0x38` (the mover's `move_to_point`); a unit
    /// without speed away from the target fails at the start (state 13), which shows the
    /// standing sprite as an arrival does.
    #[test]
    fn a_move_to_an_object_holds_its_place_and_a_unit_without_speed_fails_standing() {
        let mut world = Spatial::new([120, 120]);
        world.insert(9186, sheep(35.0, 40.0));
        let me = [30.0, 40.0, 0.0];
        let mut walker = crate::gathering::hunt::tests::walker();
        walker.stats.stand_speed = 0.5;
        let far = MoveToAction::to_target(&world, me, &mut walker, 9186, 0.0, None).unwrap();
        assert_eq!(walker.move_to_point, Some([35.0, 40.0, 0.0]));
        assert_eq!(far.point(Some(&walker)), Some([35.0, 40.0, 0.0]));
        assert_eq!(walker.stats.speed, 0.8);
        walker.stats.speed = 0.0;
        let stuck = MoveToAction::to_target(&world, me, &mut walker, 9186, 0.0, None).unwrap();
        assert_eq!(stuck.state, MoveTo::Failed);
        assert!(stuck.shows_standing());
        assert_eq!(walker.stats.speed, 0.5);
    }
}
