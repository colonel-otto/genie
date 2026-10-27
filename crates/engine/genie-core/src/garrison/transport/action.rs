//! The Transport action's slots ([`TransportAction`]): its start, state, move, stop, work and
//! update, its walk and its test beside the shore.

use crate::GenieError;
use crate::garrison::transport::{
    Changed, DONE, FAILED, IDLE, LAND_FLAGS, MOVE_RANGE, NEAR, NONE, SAIL, SAIL_TO_UNLOAD,
    SHORE_MARGIN, Ship, Shore, TransportAction, TransportStep, UNLOAD, UNLOAD_REACH_SQUARED,
    WATER_FLAGS,
};
use crate::math::conversions::cvttss2si;
use crate::movement::PathAsk;
use crate::movement::move_to::{Goal, MoveTo, Walk, start_walk_asked, walk_or_give_up};

impl TransportAction {
    /// The unload command's constructor (`FUN_140aa3ab0` with no task, `+0x48`): its point the
    /// unit's place `at`, state 0 and `+0x34` −1 (the base's, `FUN_140a73e90`), `+0x68` clear.
    #[must_use]
    pub const fn new(at: [f32; 3]) -> Self {
        Self::on_task(None, at)
    }

    /// The constructor (`FUN_140aa3ab0`) on `task` (`+0x48`), the action list's factory's for a
    /// task of type 12 at the work command's point `at` (`+0x38`..`+0x40`): state 0 and `+0x34`
    /// −1 (the base's, `FUN_140a73e90`), `+0x68` clear.
    #[must_use]
    pub const fn on_task(task: Option<crate::objects::masters::TaskId>, at: [f32; 3]) -> Self {
        Self {
            state: 0,
            mode: -1,
            point: at,
            move_to: None,
            retarget: false,
            task,
        }
    }

    /// Slot 5 (`FUN_140aa3e10`), the start: `first` (the action list's add passes 1) with a
    /// point not below 0 on either axis (`COMISS`/`JA`, `JBE`: a NaN is not) sails there
    /// (`set_state(11)`); else it idles (`set_state(2)`).
    pub fn start(&mut self, first: bool, ship: &Ship, shore: &mut impl Shore) -> Changed {
        let [x, y, _] = self.point;
        let sails = first
            && crate::math::fmath::at_most_or_unordered(0.0, x)
            && crate::math::fmath::at_most_or_unordered(0.0, y);
        self.set_state(if sails { SAIL } else { IDLE }, ship, shore)
    }

    /// `set_state` (slot 16, `FUN_140aa42a0`): its sub-actions go first (`FUN_140a98620`: a
    /// `MoveTo`'s teardown stops the unit, [`Changed::stopped`]), then the state. State 2 shows
    /// the master's standing sprite ([`Changed::stands`]). States 4 and 11 sail: the point
    /// becomes the shore point on the way from the unit's place ([`Shore::shore_point`]) when
    /// there is one; then a quiet `MoveTo` to the point within [`MOVE_RANGE`] (`FUN_140aa0d30`
    /// with no task: the master's `+0x748` sprite; `FUN_140a7a170` with 1) joins the list
    /// (`FUN_140a977e0`, for a unit in a state below 3) and starts (`0x140aa2100`: a point is
    /// never within its start's gap, so state 3, or 13 for a unit without speed, which a quiet
    /// `MoveTo` shows no sprite for).
    pub fn set_state(&mut self, state: u8, ship: &Ship, shore: &mut impl Shore) -> Changed {
        let mut changed = Changed {
            stopped: self.move_to.take().is_some(),
            stands: false,
        };
        self.state = state;
        match state {
            IDLE => changed.stands = true,
            SAIL_TO_UNLOAD | SAIL => {
                let [ux, uy, _] = ship.position;
                let to = [self.point[0], self.point[1]];
                if let Some([sx, sy]) = shore.shore_point([ux, uy], to) {
                    self.point[0] = sx;
                    self.point[1] = sy;
                }
                self.move_to = Some(if ship.moves {
                    MoveTo::Start
                } else {
                    MoveTo::Failed
                });
            }
            _ => {}
        }
        changed
    }

    /// Slot 8 (`FUN_140aa3e40`), a move to `point` (x, y and the height; the object it names
    /// is not read): the point it sails to, `+0x34` 0, then `set_state(11)`; it takes the move
    /// (1). Its sprite field (`+0x58`, the master's `+0x748` or standing sprite) is the
    /// interface's.
    pub fn move_to(&mut self, point: [f32; 3], ship: &Ship, shore: &mut impl Shore) -> Changed {
        self.point = point;
        self.mode = 0;
        self.set_state(SAIL, ship, shore)
    }

    /// Slot 7 (`FUN_140aa4620`), the stop: `set_state(2)`.
    pub fn stop(&mut self, ship: &Ship, shore: &mut impl Shore) -> Changed {
        self.set_state(IDLE, ship, shore)
    }

    /// Slot 9 (`FUN_140aa4830`), the work order on `point` (x, y and the height), `by_self` when
    /// the object it names is the unit itself (the unload command's); `+0x34` becomes 1. Near
    /// enough to unload there — a tile of the zone of the point's tile (`CVTTSS2SI`,
    /// [`Shore::zone`]) within [`NEAR`] of the unit ([`Shore::near`], exactly 1), and the point
    /// within [`UNLOAD_REACH_SQUARED`] of the unit's footprint on the ground (each axis' gap past
    /// the master's radius, `MAXSS` with 0: a NaN gap stays; `COMISS`/`JC`: a NaN is not
    /// within) — it takes the point and unloads (`set_state(6)`). Else a point below 0 on either
    /// axis (`COMISS`/`JC`: a NaN too) changes nothing; any other becomes the point, and the
    /// unit sails there to unload (`set_state(4)`) from the unit itself, else just sails (11).
    ///
    /// # Errors
    ///
    /// What the map reads give.
    pub fn work(
        &mut self,
        point: [f32; 3],
        by_self: bool,
        ship: &Ship,
        shore: &mut impl Shore,
    ) -> Result<Changed, GenieError> {
        self.mode = 1;
        let [x, y, _] = point;
        let zone = shore.zone(cvttss2si(x), cvttss2si(y))?;
        let [ux, uy, _] = ship.position;
        // MAXSS 0, gap: the gap when it is not below 0, a NaN gap too
        let dx = crate::math::fmath::maxss(0.0, (x - ux).abs() - ship.radius[0]);
        let dy = crate::math::fmath::maxss(0.0, (y - uy).abs() - ship.radius[1]);
        let near = shore.near(zone as u8, NEAR)?;
        let within =
            !crate::math::fmath::below_or_unordered(UNLOAD_REACH_SQUARED, dy * dy + dx * dx);
        let state = if near && within {
            UNLOAD
        } else if crate::math::fmath::below_or_unordered(x, 0.0)
            || crate::math::fmath::below_or_unordered(y, 0.0)
        {
            return Ok(Changed::default());
        } else if by_self {
            SAIL_TO_UNLOAD
        } else {
            SAIL
        };
        self.point = point;
        Ok(self.set_state(state, ship, shore))
    }

    /// One update (slot 6, `FUN_140aa4640`) after its `MoveTo`'s (`walked`, the code of
    /// [`TransportAction::walk`], which only states 4 and 11 read). State 1 is done (1). States 4
    /// and 11: any end of the `MoveTo` (codes 1 to 5; without one the list answers 1) unloads
    /// (4: `set_state(6)`) or idles (11: `set_state(2)`). State 13 tells the unit
    /// it failed ([`TransportStep::failed`]), idles and returns 3. State 6 unloads:
    /// - with a point, its zone's tile ([`Shore::zone`]) must lie within [`NEAR`] of the unit
    ///   ([`Shore::near`]), else the owner hears that it cannot unload there
    ///   ([`TransportStep::cannot_unload`]); `+0x68` clears either way.
    /// - with units inside, a unit beside the shore ([`TransportAction::next_to_shore`]) unloads
    ///   ([`TransportStep::unload`], the caller's, then [`TransportAction::unloaded`]); one that
    ///   is not, over water at the point (`FUN_1413fbf30`, `FUN_1413fc2b0`: the terrain table's
    ///   bits 0 to 2 at the point's tile) waits there, else the owner hears that it cannot unload.
    /// - unless the unload sails on (`+0x68`), the action idles (`set_state(2)`).
    ///
    /// # Errors
    ///
    /// What the map reads and the `MoveTo` give.
    pub fn update(
        &mut self,
        walked: u8,
        ship: &Ship,
        shore: &mut impl Shore,
    ) -> Result<TransportStep, GenieError> {
        let mut step = TransportStep::default();
        match self.state {
            DONE => step.code = 1,
            SAIL_TO_UNLOAD | SAIL => {
                if (1..=5).contains(&walked) {
                    let next = if self.state == SAIL { IDLE } else { UNLOAD };
                    step.changed = self.set_state(next, ship, shore);
                }
            }
            FAILED => {
                step.failed = true;
                step.changed = self.set_state(IDLE, ship, shore);
                step.code = 3;
            }
            UNLOAD => {
                let [x, y, _] = self.point;
                let mut reaches = true;
                if x == NONE {
                    self.retarget = false;
                } else {
                    let zone = shore.zone(cvttss2si(x), cvttss2si(y))?;
                    self.retarget = false;
                    reaches = shore.near(zone as u8, NEAR)?;
                }
                if !reaches {
                    step.cannot_unload = true;
                } else if ship.loaded {
                    if self.next_to_shore(ship, shore)? {
                        step.unload = true;
                        return Ok(step);
                    }
                    let over_water = x != NONE
                        && shore.flags(cvttss2si(x) as i16, cvttss2si(y) as i16) & WATER_FLAGS != 0;
                    step.cannot_unload = !over_water;
                }
                step.changed = self.unloaded(ship, shore);
            }
            _ => {}
        }
        Ok(step)
    }

    /// The end of state 6's update after the unload ([`TransportStep::unload`]): unless the
    /// unload sails on (`+0x68`), `set_state(2)`.
    pub fn unloaded(&mut self, ship: &Ship, shore: &mut impl Shore) -> Changed {
        if self.retarget {
            return Changed::default();
        }
        self.set_state(IDLE, ship, shore)
    }

    /// The update of its `MoveTo` sub-action (`FUN_140a992b0`) with `walk` and what the unit's
    /// AI asks of the path (`ask`, `0x140aa2a70` in state 3), as [`crate::ownership::artifact::Artifact`]
    /// runs its own: state 3 asks for its path to the point within [`MOVE_RANGE`] (quiet), state
    /// 4 moves, a motion step that gives up fails it (`set_state(13)`: only a human player's
    /// villager waits); its code: 1 once it ended at the point, 3 once it failed, 1 without one
    /// ("There is no list"), else 0.
    ///
    /// # Errors
    ///
    /// A walk the `MoveTo` needs and has not got, and what the motion step does not model.
    pub fn walk(&mut self, walk: Option<Walk<'_>>, ask: PathAsk) -> Result<u8, GenieError> {
        let need = || GenieError::unreachable("a sailing Transport without its walk");
        match self.move_to {
            None | Some(MoveTo::Arrived) => return Ok(1),
            Some(MoveTo::Failed) => return Ok(3),
            Some(MoveTo::Start) => {
                let goal = Goal::Point(self.point);
                let state = start_walk_asked(walk.ok_or_else(need)?, goal, MOVE_RANGE, true, ask)?;
                self.move_to = Some(state.unwrap_or(MoveTo::Failed));
            }
            Some(MoveTo::Moving) => {
                let state = walk_or_give_up(walk.ok_or_else(need)?)?;
                self.move_to = Some(state.unwrap_or(MoveTo::Failed));
            }
            Some(MoveTo::Waiting) => {
                return Err(GenieError::unreachable(
                    "a Transport's MoveTo waiting (only a villager's waits)",
                ));
            }
        }
        Ok(0)
    }

    /// `FUN_140aa3f40` (HD's `next_to_drop_off_terrain`), without a task (`+0x48`; a task whose
    /// `+0x16` is −1 would answer yes): a tile about the unit's footprint grown by
    /// [`SHORE_MARGIN`] (each edge cut by `CVTTSS2SI`, kept on the map) of the point's zone
    /// ([`Shore::zone`]), or without a point (x [`NONE`]) a land tile (the terrain table's bits 3
    /// and 4, `FUN_1413fbee0` and `FUN_1413fc270`); row by row, each from its least x.
    ///
    /// # Errors
    ///
    /// What the map reads give.
    pub fn next_to_shore(&self, ship: &Ship, shore: &mut impl Shore) -> Result<bool, GenieError> {
        let [ux, uy, _] = ship.position;
        let [rx, ry] = ship.radius;
        let [w, h] = ship.map;
        let x0 = cvttss2si((ux - rx) - SHORE_MARGIN).max(0);
        let y0 = cvttss2si((uy - ry) - SHORE_MARGIN).max(0);
        let x1 = cvttss2si((ux + rx) + SHORE_MARGIN).min(w - 1);
        let y1 = cvttss2si((uy + ry) + SHORE_MARGIN).min(h - 1);
        let [x, y, _] = self.point;
        let zone = if x == NONE {
            None
        } else {
            Some(shore.zone(cvttss2si(x), cvttss2si(y))?)
        };
        for ty in y0..=y1 {
            for tx in x0..=x1 {
                let found = match zone {
                    None => shore.flags(tx as i16, ty as i16) & LAND_FLAGS != 0,
                    Some(z) => shore.zone(tx, ty)? == z,
                };
                if found {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
}
