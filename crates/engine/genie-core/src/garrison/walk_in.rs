//! The Enter action ([`EnterAction`], `RGE_Action_Enter`): its walk to its target, its wait
//! and its update.

use crate::GenieError;
use crate::garrison::meeting::Meeting;
use crate::garrison::{ENTER_NEAR, HALF_TILE};
use crate::math::rng::Stream;
use crate::movement::Me;
use crate::movement::move_to::{Goal, MoveDefaults, MoveTo, Sprites, sub_move_to_update};
use crate::objects::lifecycle::seconds;
use crate::objects::masters::Task;
use crate::objects::sprite::sprite_id;
use crate::objects::teardown::HeadCode;
use crate::objects::unit::Unit;
use crate::world::others::OthersRef;
use crate::world::step::Step;
use genie_diff::Diff;

/// The Enter action's type (`RGE_Action_Enter`, vtable `0x142ed4db8`), which word 9 adds.
pub const ENTER: i16 = 3;

/// The gap below which the Enter's `MoveTo` starts arrived (`0x140aa2100`, `DAT_142eb99cc`).
pub(super) const ARRIVED: f32 = crate::math::image::move_to::ARRIVED;

/// The Enter action's states the slice models: 1 done (inside), 2 waiting to try again, 4
/// walking to the target.
pub const ENTERED: u8 = 1;
/// The failed Enter (`set_state(13)`), whose update returns 3.
pub const FAILED: u8 = 13;
pub(super) const WAIT: u8 = 2;
pub(super) const WALK: u8 = 4;
/// `set_state(2)`'s wait before the Enter walks again, in seconds (`FUN_140a87320`, the constant
/// `0x3f800000` it stores).
pub(super) const WAIT_SECONDS: f32 = 1.0;
/// The class of siege weapons, whose own place an Enter's point keeps (`FUN_140a85f30`, the
/// short `0xd`).
pub(crate) const SIEGE_CLASS: i16 = 13;

/// An Enter's `MoveTo` of a unit without speed (`0x140aa2100`'s state 13), or of a unit without
/// movement state, which is not modelled.
pub(crate) const NO_SPEED: GenieError =
    GenieError::unsupported("an Enter's MoveTo of a unit without speed");

/// An unload of a siege building or tower (class 13: its spot search `FUN_141192d10` and the
/// unit's leave), not modelled.
pub(crate) const SIEGE_UNLOAD: GenieError =
    GenieError::unsupported("an unload of a siege building (class 13)");

/// A unit's Enter action (`RGE_Action_Enter`, type 3): it walks to a building or a transport and
/// enters it.
#[derive(Clone, Copy, Debug, PartialEq, Diff)]
pub struct EnterAction {
    /// The building or transport (`+0x20`, its id `+0x30`).
    pub target: i32,
    /// The action state (`+0x18`).
    pub state: u8,
    /// Its `MoveTo` sub-action's state.
    pub move_to: Option<MoveTo>,
    /// `+0x38`..`+0x40`: where it meets the target ([`meeting`]: a building's place; for a
    /// transport where the two meet, its height the target's at the constructor), which a
    /// target that moves leaves.
    ///
    /// [`meeting`]: crate::garrison::meeting::meeting_point
    pub point: [f32; 3],
    /// The task's work range (`+0x30`): a building's `MoveTo` stops this near, and the enter
    /// needs it.
    pub range: f32,
    /// `+0x44`: state 2's wait, in seconds.
    pub timer: f32,
    /// `+0x68`, 1 at first: a `MoveTo` that fails (code 3) asks once more.
    pub retry: bool,
    /// `+0x6c`: the task's class (its `+0x10`).
    pub task_class: i16,
    /// Its `MoveTo`'s wait after the motion step gave up (`+0x6c`..`+0x74`).
    pub move_wait: crate::movement::move_to::MoveWait,
    /// The task's moving sprite (`+0x40`), which its `MoveTo`s take; None for none.
    pub moving: Option<i16>,
}

/// What an Enter's update reads of its target.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EnterTarget {
    /// Its state (`+0x640`).
    pub state: u8,
    /// The unit's player counts its player as allied (`FUN_141663dc0`), or it is Gaia's.
    pub allied: bool,
    /// It has room for the unit (slot 0x198).
    pub room: bool,
    /// Its place.
    pub position: [f32; 3],
    /// The unit is in reach of it ([`EnterAction::in_reach`]).
    pub in_reach: bool,
    /// It is a building (type 80); else a transport.
    pub building: bool,
}

/// What a new `MoveTo` of an Enter starts by (`0x140aa2100` with 1,
/// [`EnterAction::walk_start`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WalkStart {
    /// The footprints' gap of the unit and its target (`FUN_14118f680`).
    pub gap: f32,
    /// The unit's speed.
    pub speed: f32,
}

/// What an Enter's update (`FUN_140a87a80`) reads as it starts.
#[derive(Clone, Copy, Debug)]
pub(crate) struct EnterReads {
    /// Its target, of the unit's own player or an ally; None for one the unit's player does not
    /// find.
    pub(crate) target: Option<EnterTarget>,
    /// What a new `MoveTo` starts by.
    pub(crate) start: WalkStart,
    /// The `MoveTo`'s give-up waits (a human player's villager), by the unit AI's stance.
    pub(crate) waits: bool,
    /// The unit AI's stance.
    pub(crate) stance: u8,
    /// The sprites its `MoveTo`s' constructors take for none (`FUN_140aa0d30`,
    /// `FUN_140aa0df0`).
    pub(crate) defaults: MoveDefaults,
}

/// What [`EnterAction::walk_again`] (`set_state(4)`) leaves to its caller.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Walking {
    /// The unit is in reach: the caller enters (`FUN_140a867b0`), then `set_state(1)`.
    Enter,
    /// A `MoveTo` to the target started; with a point, the transport is to move there first
    /// (its slots 0x3d8 and 0x340, [`crate::world::World::move_transport`]).
    Walk(Option<[f32; 3]>),
}

impl EnterAction {
    /// The factory's constructor (`FUN_140a855d0`): the target (an annex's main building, the
    /// caller's, as its add, slot 0x178), the point [`meeting`] gives at the target's height,
    /// the task's (`task`) range, class and moving sprite, `+0x68` 1; the start (slot 5,
    /// `FUN_140a86f20`) and its `set_state(4)` are [`EnterAction::walk_again`].
    ///
    /// [`meeting`]: crate::garrison::meeting::meeting_point
    #[must_use]
    pub const fn new(target: i32, point: [f32; 3], task: &Task) -> Self {
        Self {
            target,
            state: WALK,
            move_to: None,
            point,
            range: task.work_range,
            timer: 0.0,
            retry: true,
            task_class: task.object_group,
            move_wait: crate::movement::move_to::MoveWait::NEW,
            moving: sprite_id(task.sprites[0]),
        }
    }

    /// The sprite its `MoveTo` shows (`set_state(4)`, `FUN_140a87320` case 4): the task's moving
    /// sprite, else its constructor's default from `defaults`: `FUN_140aa0df0`'s toward a
    /// `building` or toward a transport from a point below 0, `FUN_140aa0d30`'s toward a
    /// transport's point (`UCOMISS`: a NaN coordinate is not below 0).
    #[must_use]
    pub fn move_shown(&self, building: bool, defaults: MoveDefaults) -> Option<i16> {
        if building || self.point[0] < 0.0 || self.point[1] < 0.0 {
            defaults.to_object(self.moving)
        } else {
            defaults.to_point(self.moving)
        }
    }

    /// `set_state(4)` (`FUN_140a87320` case 4) after the start's or the update's tests: its
    /// sub-actions go (`FUN_140a98620`; a `MoveTo`'s teardown stops the unit, the caller's), and
    /// its room test repeats theirs, which nothing between changes. A unit in reach of the
    /// target (`FUN_140a85c30`) enters it ([`Walking::Enter`]). Else a building's `MoveTo` walks
    /// to it within the task's range (`FUN_140aa0df0`); a transport's takes the point and code
    /// of [`meeting`] (`FUN_140a85f30`, [`Meeting::point`]: the action's `+0x38` and `+0x3c`)
    /// and walks to the transport within 0 (`FUN_140aa0d30` with range 0, then slot 0x68 with
    /// the transport: its state 3 goes to the transport's place; a point below 0 would make
    /// `FUN_140aa0df0` with the unit's slot 0x268, 0, alike), and code 1 asks the transport to
    /// move to the point. Both `MoveTo`s are quiet (`FUN_140a7a170` with 1) and leave a target
    /// that moves (`+0x61` 2); their start is [`EnterAction::walk_start`]'s.
    ///
    /// # Errors
    ///
    /// The meeting's and the start's.
    ///
    /// [`meeting`]: crate::garrison::meeting::meeting_point
    pub(crate) fn walk_again(
        &mut self,
        t: &EnterTarget,
        start: WalkStart,
        meeting: Meeting<'_>,
    ) -> Result<Walking, GenieError> {
        self.move_to = None;
        self.state = WALK;
        if t.in_reach {
            return Ok(Walking::Enter);
        }
        let code = if t.building {
            0
        } else {
            let (point, code) = meeting.point()?;
            self.point[..2].copy_from_slice(&point);
            code
        };
        self.move_to = Some(Self::walk_start(start)?);
        Ok(Walking::Walk((code == 1).then_some([
            self.point[0],
            self.point[1],
            t.position[2],
        ])))
    }

    /// The `MoveTo`'s start (`0x140aa2100` with 1): beyond [`ARRIVED`] of the target's footprint
    /// (`start.gap`) state 3, or 13 for a unit without speed ([`NO_SPEED`], not modelled); else
    /// state 1.
    ///
    /// # Errors
    ///
    /// [`NO_SPEED`].
    pub(super) fn walk_start(start: WalkStart) -> Result<MoveTo, GenieError> {
        if ARRIVED < start.gap {
            if start.speed == 0.0 {
                return Err(NO_SPEED);
            }
            return Ok(MoveTo::Start);
        }
        Ok(MoveTo::Arrived)
    }

    /// The range its `MoveTo` stops within: the task's for a building, 0 for a transport.
    pub(super) const fn walk_range(&self, building: bool) -> f32 {
        if building { self.range } else { 0.0 }
    }

    /// `set_state(2)` (`FUN_140a87320` case 2): the wait of [`WAIT_SECONDS`], and the unit shows
    /// its master's standing sprite at once (slot 0x68 with `+0x470`,
    /// [`Unit::shows_standing_sprite`]).
    ///
    /// # Errors
    ///
    /// The sprite's.
    pub(super) fn wait(&mut self, c: &mut Step<'_>, unit: &mut Unit) -> Result<(), GenieError> {
        self.move_to = None;
        self.state = WAIT;
        self.timer = WAIT_SECONDS;
        unit.shows_standing_sprite(
            c.w.masters.get(c.player),
            &c.w.graphics,
            c.w.streams.get(Stream::Sprite),
        )
    }

    /// `FUN_140a85c30`: the unit is in reach of the target: their footprints' squared gap
    /// (`FUN_14118f8c0`; `COMISS`/`JBE`: a NaN gap is not out) at most the range squared; for a
    /// range of at most 1 (`COMISS`/`JC`: a NaN range is in reach) to a target with speed (slot
    /// 0x208, above 0) the tiles about the two ([`Walls::walled_off`]) must not hold exactly
    /// two buildings.
    ///
    /// # Errors
    ///
    /// The tile test's.
    ///
    /// [`Walls::walled_off`]: crate::garrison::Walls::walled_off
    pub fn in_reach(
        range: f32,
        gap_squared: f32,
        target_speed: f32,
        walls: &crate::garrison::Walls<'_>,
    ) -> Result<bool, GenieError> {
        if range * range < gap_squared {
            return Ok(false);
        }
        if range <= ENTER_NEAR && 0.0 < target_speed {
            return Ok(!walls.walled_off()?);
        }
        Ok(true)
    }

    /// One update (slot 6, `FUN_140a87a80`) of `unit`'s Enter, a copy its caller writes back
    /// ([`Unit::enter_part`]). It first checks its target ([`EnterAction::held_target`]). State
    /// 1 is done; state 2 waits its timer (by the world's `dt`), then walks again
    /// ([`EnterAction::again`]); state 4 walks ([`EnterAction::walk_on`]). State 13 returns 3,
    /// which deletes it as 1 does. Its code: 1 or 3 delete it.
    ///
    /// # Errors
    ///
    /// A target the update does not hold (state 11), a state not modelled, and what the walk,
    /// the enter and the transport's move do not model.
    pub(crate) fn update(
        &mut self,
        c: &mut Step<'_>,
        unit: &mut Unit,
        me: Me,
        reads: &EnterReads,
    ) -> Result<HeadCode, GenieError> {
        let t = Self::held_target(reads.target)?;
        match self.state {
            // state 1 returns 1, state 13 returns 3: either way the list removes the action
            ENTERED | FAILED => return Ok(HeadCode::Deletes),
            WAIT => {
                self.timer -= seconds(c.dt);
                if self.timer <= 0.0 {
                    let _ = self.again(c, unit, &t, reads)?;
                }
            }
            WALK => self.walk_on(c, unit, me, &t, reads)?,
            _ => return Err(GenieError::unsupported("an Enter state not modelled")),
        }
        Ok(HeadCode::Keeps)
    }

    /// State 4: a target off the action's point ([`EnterAction::target_moved`]) walks it again;
    /// then the `MoveTo` updates ([`sub_move_to_update`] within the task's range, quiet, with
    /// the reads' give-up; its start shows [`EnterAction::move_shown`]): codes 1 and 2 enter in
    /// reach, a unit that entered in the walk again enters again ([`Unit::entered_again`]), else
    /// they wait (`set_state(2)`); code 3 (a `MoveTo` that failed) walks again once (`+0x68`),
    /// then waits; codes 4 and 5 let the target go (state 11), which is not modelled.
    ///
    /// # Errors
    ///
    /// The walk's, the walk again's, the enter's and the sprite's.
    fn walk_on(
        &mut self,
        c: &mut Step<'_>,
        unit: &mut Unit,
        me: Me,
        t: &EnterTarget,
        reads: &EnterReads,
    ) -> Result<(), GenieError> {
        let walked_again = self.target_moved(t);
        let entered = walked_again && self.again(c, unit, t, reads)?;
        let code = self.move_to_update(c, unit, me, t, reads)?;
        match code {
            // the walk again's enter put the unit inside, at the target's place: still in reach,
            // it enters again
            1 | 2 if entered => self.on_unit(unit, |unit| unit.entered_again(c)),
            1 | 2 if t.in_reach => self.enter_at_once(c, unit),
            // a failed MoveTo is none a walk again made in this update
            3 if self.retry => {
                self.retry = false;
                self.again(c, unit, t, reads).map(|_| ())
            }
            1..=3 => self.wait(c, unit),
            _ => Ok(()),
        }
    }

    /// The `MoveTo`'s update ([`sub_move_to_update`]) toward the target within
    /// [`EnterAction::walk_range`], quiet, its start showing [`EnterAction::move_shown`]; its
    /// code.
    ///
    /// # Errors
    ///
    /// The walk's and the sprite's.
    fn move_to_update(
        &mut self,
        c: &mut Step<'_>,
        unit: &mut Unit,
        me: Me,
        t: &EnterTarget,
        reads: &EnterReads,
    ) -> Result<u8, GenieError> {
        let range = self.walk_range(t.building);
        let id = (self.move_to == Some(MoveTo::Start))
            .then(|| self.move_shown(t.building, reads.defaults));
        let (mut walk, beside) = c.w.walk_beside(c.pace(), &mut unit.body, me);
        let mut sprites = Sprites {
            sprite: &mut unit.sprite,
            rng: beside.sprite_rng,
            graphics: beside.graphics,
        };
        let shown = id.map(|id| sprites.shown(id)).transpose()?;
        sub_move_to_update(
            (&mut self.move_to, &mut self.move_wait),
            &mut walk,
            Goal::Target(self.target),
            (range, true),
            (reads.waits, reads.stance),
            shown,
        )
    }

    /// The update's test of its target first (`FUN_140a87a80`): one gone, dead (state 3 or
    /// more) or of an unallied player, or without room for the unit, makes the action let it go
    /// and turn to state 11, which is not modelled.
    pub(super) const fn held_target(
        target: Option<EnterTarget>,
    ) -> Result<EnterTarget, GenieError> {
        let Some(t) = target else {
            return Err(GenieError::unsupported(
                "an Enter whose building is gone (state 11)",
            ));
        };
        if 3 <= t.state || !t.allied {
            return Err(GenieError::unsupported(
                "an Enter whose building died or is not allied (state 11)",
            ));
        }
        if !t.room {
            return Err(GenieError::unsupported(
                "an Enter whose building has no room (state 11)",
            ));
        }
        Ok(t)
    }

    /// State 4's test of its target: off the action's point (`FUN_140a7bbf0`: any of x, y and
    /// height not equal) by more than [`HALF_TILE`] on an axis (`FUN_140a7bd10`: `COMISS`/`JA`,
    /// a NaN is not).
    pub(super) fn target_moved(&self, t: &EnterTarget) -> bool {
        let [x, y, z] = t.position;
        let moved = |d: f32| HALF_TILE < d.abs();
        t.position != self.point
            && (moved(x - self.point[0]) || moved(y - self.point[1]) || moved(z - self.point[2]))
    }

    /// The update's `set_state(4)` ([`EnterAction::walk_again`]): its `MoveTo`'s teardown stops
    /// the unit (slot 0x5d0), then a unit in reach enters at once (`FUN_140a867b0`, then
    /// `set_state(1)`: [`EnterAction::enter_at_once`]). A transport's code 1 sends it to the
    /// point at once: it tests that it reaches within 1 of the point's tile (slot 0x3d8,
    /// `FUN_14114ffc0` with the point truncated), then moves there (slot 0x340,
    /// [`crate::world::World::move_transport`], the unit lent to the world). A transport that
    /// cannot reach it makes `set_state(13)`, which is not modelled ([`UNREACHED`]). True when
    /// the unit entered.
    ///
    /// # Errors
    ///
    /// The stop's, the walk again's, the enter's and the transport's.
    pub(super) fn again(
        &mut self,
        c: &mut Step<'_>,
        unit: &mut Unit,
        t: &EnterTarget,
        reads: &EnterReads,
    ) -> Result<bool, GenieError> {
        if self.move_to.is_some()
            && let Some(m) = unit.body.mover_mut()
        {
            m.stop()?;
        }
        let walking = {
            let w = &mut *c.w;
            let (tp, target) = OthersRef::of(&w.players, c.player)
                .find(self.target)
                .ok_or(GenieError::unreachable(
                    "an Enter's held target gone in its update",
                ))?;
            let master = target.master(&w.masters, tp)?;
            let meeting = Meeting {
                spatial: &w.spatial,
                zones: &mut w.zones,
                target,
                master,
                room: t.room,
                unit,
            };
            self.walk_again(t, reads.start, meeting)?
        };
        match walking {
            Walking::Enter => {
                self.enter_at_once(c, unit)?;
                return Ok(true);
            }
            Walking::Walk(Some(at)) => {
                let target = self.target;
                self.on_unit(unit, |unit| c.lend(unit, |w| w.move_transport(target, at)))?;
            }
            Walking::Walk(None) => {}
        }
        Ok(false)
    }

    /// The enter at once (`FUN_140a867b0` into the target, then `set_state(1)`:
    /// [`Unit::enter_in_update`]) on the unit's Enter: this copy written back first and read back
    /// after ([`EnterAction::on_unit`]).
    ///
    /// # Errors
    ///
    /// The enter's.
    fn enter_at_once(&mut self, c: &mut Step<'_>, unit: &mut Unit) -> Result<(), GenieError> {
        let target = self.target;
        self.on_unit(unit, |unit| unit.enter_in_update(c, target))
    }

    /// Runs `f` on `unit` with this copy of its Enter written back to it (a routine that reads or
    /// changes the unit's Enter, or lends the unit to the world), then takes the Enter back as
    /// `f` left it.
    ///
    /// # Errors
    ///
    /// `f`'s, or a unit whose Enter `f` took.
    fn on_unit<R>(
        &mut self,
        unit: &mut Unit,
        f: impl FnOnce(&mut Unit) -> Result<R, GenieError>,
    ) -> Result<R, GenieError> {
        unit.actions.set_enter(Some(*self));
        let answer = f(unit);
        *self = *unit
            .actions
            .enter()
            .ok_or(GenieError::unreachable("an Enter gone in its own update"))?;
        answer
    }
}
