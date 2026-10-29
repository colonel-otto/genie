//! The walks to a point and the waits: back to search (state 11), back to reseed (0xf2), for
//! a farm (0xf3), the search (3) and a move's walk (15).

use crate::GenieError;
use crate::combat::damage::Victim;
use crate::gathering::gather::update::UNMOVING;
use crate::gathering::gather::{
    FARM_WAIT, FARM_WAIT_TIME, FARM_WAITERS, GatherAction, GatherReads, IDLE, NO_GATHER_POINT,
    Next, RESEED_WALK, SEARCH, SEARCH_WAIT, STOP_FARMS, TRAP_WAITERS, WALK_BACK,
};
use crate::math::rng::Well512;
use crate::movement::move_to::{MoveTo, MoveWait};
use crate::movement::{Me, Mover};
use crate::objects::lifecycle::seconds;
use crate::objects::masters::class;
use crate::objects::sprite::ActiveSprite;
use crate::objects::sprite::new_sprite;
use crate::objects::unit::Unit;
use crate::orders::action_state::DONE;
use crate::world::step::Step;

/// Whether a gatherer of `master` whose Gather has the target class `target_type` waits for
/// another farm where another would search (`set_state(0xf3)`, `0x140a8e1d0`): the farmers'
/// masters, and the fishing ships' on a farm-class target (a fish trap).
#[must_use]
pub fn waits_for_a_farm(master: i16, target_type: i32) -> bool {
    FARM_WAITERS.contains(&master)
        || (TRAP_WAITERS.contains(&master) && target_type == i32::from(class::FARM))
}

impl GatherAction {
    /// `set_state(11)` (`0x140a8cd40` case 0xb) with its target's radius on x (`+0x5d8`; none
    /// for a stop farm, masters 50 and 1187): the sub-actions go, the timer clears and a quiet
    /// `MoveTo` to the point within that radius plus the task's range starts (`FUN_140aa0d30`).
    ///
    /// # Errors
    ///
    /// A unit that cannot move (`set_state(13)`).
    pub(super) fn set_walk_back(
        &mut self,
        radius: f32,
        mover: &mut Mover,
    ) -> Result<(), GenieError> {
        self.walk_back(radius, mover)
    }

    /// [`GatherAction::set_walk_back`] for the caller (state 8's end without a target: no
    /// target's radius).
    ///
    /// # Errors
    ///
    /// A unit that cannot move (`set_state(13)`).
    pub fn walk_back(&mut self, radius: f32, mover: &mut Mover) -> Result<(), GenieError> {
        self.walk_to_point(WALK_BACK, radius, mover)
    }

    /// `set_state(0xf2)` (`0x140a8cd40` case 0xf2): the point takes `mGatherX`, `mGatherY`
    /// unless `mGatherX` is −1 (`UCOMISS`: a NaN takes them), then case 0xb's walk to it within
    /// `radius` plus the task's range ([`GatherAction::walk_back`]'s, in state 0xf2).
    ///
    /// # Errors
    ///
    /// A unit that cannot move (`set_state(13)`).
    pub(super) fn begin_reseed_walk(
        &mut self,
        radius: f32,
        mover: &mut Mover,
    ) -> Result<(), GenieError> {
        if self.gather_at[0] != NO_GATHER_POINT {
            self.point[0] = self.gather_at[0];
            self.point[1] = self.gather_at[1];
        }
        self.walk_to_point(RESEED_WALK, radius, mover)
    }

    /// `set_state` in `state` 11 or 0xf2 (case 0xb's walk): the sub-actions go, the timer
    /// clears and a quiet `MoveTo` to the point within `radius` plus the task's range starts
    /// (`FUN_140aa0d30`).
    pub(super) fn walk_to_point(
        &mut self,
        state: u8,
        radius: f32,
        mover: &mut Mover,
    ) -> Result<(), GenieError> {
        self.tear_down(Some(mover))?;
        self.state = state;
        self.timer = 0.0;
        if mover.stats.speed == 0.0 {
            return Err(GenieError::unsupported(
                "a MoveTo of a unit without speed (state 13)",
            ));
        }
        self.move_wait = MoveWait::default();
        self.walk_to = Some(self.point);
        self.walk_range = radius + self.task.range;
        self.move_to = Some(MoveTo::Start);
        Ok(())
    }

    /// State 11's update (`0x140a8e1d0` case 10): its `MoveTo` walks; once it ended, arrived or
    /// failed (codes 1, 2, 3, 5), the gatherer searches from there (`set_state(3)`,
    /// [`GatherAction::begin_search`]).
    pub(super) fn update_walk_back(
        &mut self,
        c: &mut Step<'_>,
        unit: &mut Unit,
        me: Me,
        r: &GatherReads,
    ) -> Result<Next, GenieError> {
        match self.move_to {
            Some(MoveTo::Arrived | MoveTo::Failed) => self.begin_search(c, unit, r),
            Some(_) => {
                self.step_move(c, unit, me, r)?;
                Ok(Next::Stay)
            }
            None => Err(GenieError::unsupported(
                "a Gather in state 11 without its MoveTo",
            )),
        }
    }

    /// `set_state(0xf3)` (case 0xf3; `FUN_140a98620` first): as the search's
    /// ([`GatherAction::begin_search`], the same notify) with a timer of a second.
    pub(super) fn begin_farm_wait(
        &mut self,
        c: &mut Step<'_>,
        unit: &mut Unit,
        r: &GatherReads,
    ) -> Result<Next, GenieError> {
        self.tear_down(unit.body.mover_mut())?;
        self.state = FARM_WAIT;
        self.timer = FARM_WAIT_TIME;
        Self::show(c, unit, self.stand_sprite(unit, r));
        Ok(Next::Search)
    }

    /// State 0xf3's update (`0x140a8e1d0` case 0xf2): the timer counts up by the world's dt
    /// while below 0; once at 0 a gatherer that never gathered (`+0x80`) at a farm (its target
    /// type 49) another farmer works too (`+0x644` above 1) walks back to its point to search
    /// (`set_state(11)` with the farm's radius, none for a stop farm, then its target goes:
    /// slot 0x68 with none); one that never gathered otherwise idles (`set_state(2)`); one that
    /// gathered walks back to where it gathered to reseed (`set_state(0xf2)`). A timer at 0 or
    /// more does nothing.
    pub(super) fn update_farm_wait(
        &mut self,
        c: &mut Step<'_>,
        unit: &mut Unit,
        target: Option<&Victim>,
        r: &GatherReads,
    ) -> Result<Next, GenieError> {
        if 0.0 <= self.timer {
            return Ok(Next::Stay);
        }
        self.timer += seconds(c.dt);
        if self.timer < 0.0 {
            return Ok(Next::Stay);
        }
        let radius = target.map_or(0.0, |t| {
            if STOP_FARMS.contains(&t.master_id) {
                0.0
            } else {
                t.radius[0]
            }
        });
        if !self.worked {
            if self.target_type == i32::from(class::FARM) && target.is_some_and(|t| 1 < t.workers) {
                let mover = unit.body.mover_mut().ok_or(UNMOVING)?;
                self.set_walk_back(radius, mover)?;
                self.target = -1;
                return Ok(Next::Stay);
            }
            self.tear_down(unit.body.mover_mut())?;
            self.set_idle(c, unit, r);
            return Ok(Next::Stay);
        }
        let mover = unit.body.mover_mut().ok_or(UNMOVING)?;
        self.begin_reseed_walk(radius, mover)?;
        Ok(Next::Stay)
    }

    /// State 0xf2's update (`0x140a8e1d0` case 0xf1): its `MoveTo` walks; once it arrived (codes
    /// 1 and 2: the list drops it, its destructor stops the unit) a target in a state of 2 or
    /// less, or 6, is faced again (`FUN_1411944f0` false: `set_state(6)`); else, and after a
    /// failed walk (code 3), the caller reseeds ([`Next::Reseed`]). An empty list's update
    /// answers none of the codes: nothing happens.
    pub(super) fn update_reseed_walk(
        &mut self,
        c: &mut Step<'_>,
        unit: &mut Unit,
        me: Me,
        target: Option<&Victim>,
        r: &GatherReads,
    ) -> Result<Next, GenieError> {
        match self.move_to {
            Some(MoveTo::Arrived) => {
                self.move_to = None;
                self.walk_to = None;
                unit.body.mover_mut().ok_or(UNMOVING)?.stop()?;
                if let Some(t) = target
                    && !(2 < t.state && t.state != 6)
                {
                    return self.set_face(c, unit, t, r);
                }
                Ok(Next::Reseed { arrived: true })
            }
            Some(MoveTo::Failed) => Ok(Next::Reseed { arrived: false }),
            Some(_) => {
                self.step_move(c, unit, me, r)?;
                Ok(Next::Stay)
            }
            None => Ok(Next::Stay),
        }
    }

    /// State 0xf2's end without a reseed ([`Next::Reseed`]): the sub-actions go
    /// (`FUN_140a98620`); after a walk that arrived a computer player's gatherer is done
    /// (`set_state(1)`: the timer clears, the unit stands), any other idles (`set_state(2)`,
    /// [`GatherAction::set_idle`]: the targets it lets go are returned for the caller's
    /// release). The unit stands with `stand` (`FUN_140a8b6b0`).
    ///
    /// # Errors
    ///
    /// The unit's stop that the motion step does not model.
    pub(crate) fn after_reseed(
        &mut self,
        arrived: bool,
        computer: bool,
        mover: Option<&mut Mover>,
        stand: Option<crate::objects::sprite::Graphic>,
        sprite: &mut Option<ActiveSprite>,
        rng: &mut Well512,
    ) -> Result<[i32; 2], GenieError> {
        let mut mover = mover;
        self.tear_down(mover.as_deref_mut())?;
        self.timer = 0.0;
        if let Some(s) = stand {
            new_sprite(sprite, mover.map(|m| &mut m.stats), s, rng);
        }
        if arrived && computer {
            self.state = DONE;
            return Ok([-1; 2]);
        }
        self.state = IDLE;
        let released = [self.target, self.carry_to];
        self.target = -1;
        self.carry_to = -1;
        self.point = [-1.0; 3];
        Ok(released)
    }

    /// `set_state(3)` (case 3; `FUN_140a98620` first: the `MoveTo` goes and stops the unit): the
    /// caller posts the unit AI's notify ([`Next::Search`]), the timer waits 10 seconds and the
    /// unit stands ([`GatherAction::stand_sprite`]).
    pub(super) fn begin_search(
        &mut self,
        c: &mut Step<'_>,
        unit: &mut Unit,
        r: &GatherReads,
    ) -> Result<Next, GenieError> {
        self.tear_down(unit.body.mover_mut())?;
        self.state = SEARCH;
        self.timer = SEARCH_WAIT;
        Self::show(c, unit, self.stand_sprite(unit, r));
        Ok(Next::Search)
    }

    /// `set_state(1)` (case 1): the timer clears and the unit stands.
    pub(super) fn set_done(&mut self, c: &mut Step<'_>, unit: &mut Unit, r: &GatherReads) {
        self.state = DONE;
        self.timer = 0.0;
        Self::show(c, unit, self.stand_sprite(unit, r));
    }

    /// The sprite the gatherer stands with (`FUN_140a8b6b0`): the task's carrying sprite while
    /// it carries at least 1 of the task's resource, else its master's standing sprite.
    pub(super) const fn stand_sprite(
        &self,
        unit: &Unit,
        r: &GatherReads,
    ) -> Option<crate::objects::sprite::Graphic> {
        let resource = self.task.resource;
        let (kind, held) = (unit.life.attribute_type, unit.life.amount);
        match r.sprites.carrying {
            Some(c) if resource != -1 && kind == resource && 1 <= held as i32 => Some(c),
            _ => r.sprites.standing,
        }
    }

    /// State 15's update (`0x140a8e1d0` case 0xe): its `MoveTo` walks; once it ended (arrived,
    /// codes 1 and 2) a computer player's gatherer is done (`set_state(1)`), a human's idles
    /// (`set_state(2)`); a failed walk (code 3) idles both.
    pub(super) fn update_moved(
        &mut self,
        c: &mut Step<'_>,
        unit: &mut Unit,
        me: Me,
        r: &GatherReads,
    ) -> Result<Next, GenieError> {
        match self.move_to {
            Some(MoveTo::Arrived | MoveTo::Failed) => {
                let arrived = self.move_to == Some(MoveTo::Arrived);
                self.move_to = None;
                self.walk_to = None;
                unit.body.mover_mut().ok_or(UNMOVING)?.stop()?;
                if arrived && r.computer {
                    self.set_done(c, unit, r);
                } else {
                    self.set_idle(c, unit, r);
                }
                Ok(Next::Stay)
            }
            Some(_) => {
                self.step_move(c, unit, me, r)?;
                Ok(Next::Stay)
            }
            None => Err(GenieError::unsupported(
                "a Gather in state 15 without its MoveTo",
            )),
        }
    }
}
