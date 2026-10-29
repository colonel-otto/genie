//! One update (slot 6, `0x140a8e1d0`) in states 4, 6 and 7: the walk's `MoveTo`, the units
//! in the way and the teardown.

use crate::GenieError;
use crate::combat::damage::Victim;
use crate::gathering::gather::{
    FACE, FARM_WAIT, GatherAction, GatherReads, IDLE, MOVED, Next, REGATHERED, RESEED_WALK, SEARCH,
    SEARCH_WAIT, STOP_FARMS, StopRate, WAIT_GAP, WAIT_WORKERS, WALK, WALK_BACK, WORK,
    waits_for_a_farm,
};
use crate::gathering::hunt::walk_sprite;
use crate::math::rng::Stream;
use crate::movement::move_to::walk_or_give_up;
use crate::movement::move_to::{Goal, MoveTo, Shown, start_walk_shown, target_moved};
use crate::movement::{Me, Mover, PathAsk};
use crate::objects::lifecycle::seconds;
use crate::objects::masters::class;
use crate::objects::unit::Unit;
use crate::orders::action_state::DONE;
use crate::world::step::Step;

/// A gatherer without movement state, which the Gather's walks and turns need.
pub(super) const UNMOVING: GenieError =
    GenieError::unsupported("a gatherer without movement state");

impl GatherAction {
    /// Slot 0x78 (`FUN_140a78060`, the base action's, [`crate::gathering::hunt::asked_drop_off`] on the
    /// task's resource): true when the unit's load asks for `set_state(8)`; false for a load of
    /// another resource (the unit AI takes notify 0x21c, the caller's).
    #[must_use]
    pub fn drop_off_asked(&self, held_kind: i16, attributes: usize) -> bool {
        crate::gathering::hunt::asked_drop_off(self.task.resource, held_kind, attributes)
    }

    /// The teardown (`FUN_140a73fc0`) and `set_state`'s first step (`FUN_140a98620`): the
    /// `MoveTo` goes and stops the unit (slot 0x5d0); the caller releases the targets.
    ///
    /// # Errors
    ///
    /// The unit's stop that the motion step does not model.
    pub fn tear_down(&mut self, mover: Option<&mut Mover>) -> Result<(), GenieError> {
        self.walk_to = None;
        if self.move_to.take().is_some()
            && let Some(m) = mover
        {
            m.stop()?;
        }
        Ok(())
    }

    /// The sprite the walks show (`FUN_140a8b660`, slot 0x68 with it): the task's carrying sprite
    /// while the unit carries at least 1 of the task's resource, else its moving one.
    pub(super) const fn walk_sprite(
        &self,
        unit: &Unit,
        r: &GatherReads,
    ) -> Option<crate::objects::sprite::Graphic> {
        walk_sprite(
            self.task.resource,
            unit.life.attribute_type,
            unit.life.amount,
            &r.sprites,
        )
    }

    /// One update (slot 6, `0x140a8e1d0`) of `unit`'s Gather in states other than 8, `target`
    /// the target the caller holds (None once it went). State 4 walks the `MoveTo`
    /// (`FUN_140a992b0`); its end (codes 1 and 2) is `set_state(6)`. State 6 faces the target
    /// (`FUN_141162db0`) and, while it holds something, `set_state(7)`. State 7 gathers
    /// ([`GatherAction::gather`]). Without a target a loaded gatherer takes its load off
    /// ([`Next::Drop`]). The world's dt (`c.dt`) runs its timers.
    ///
    /// # Errors
    ///
    /// What the update does not model.
    pub(crate) fn update(
        &mut self,
        c: &mut Step<'_>,
        unit: &mut Unit,
        me: Me,
        target: Option<&mut Victim>,
        r: &GatherReads,
    ) -> Result<Next, GenieError> {
        // a farm's time per stop follows the gatherer's work rate, every update
        if target
            .as_deref()
            .is_some_and(|t| STOP_FARMS.contains(&t.master_id))
        {
            self.time_per_stop = StopRate {
                work_rate: r.work_rate,
                capacity: r.capacity,
                productivity: r.stop_productivity,
            }
            .time_per_stop(self.task.work_value, self.stops);
        }
        let dt = seconds(c.dt);
        match self.state {
            DONE => return Ok(Next::Done),
            IDLE => return Ok(Next::Stay),
            MOVED => return self.update_moved(c, unit, me, r),
            WALK_BACK => return self.update_walk_back(c, unit, me, r),
            FARM_WAIT => return self.update_farm_wait(c, unit, target.as_deref(), r),
            RESEED_WALK => return self.update_reseed_walk(c, unit, me, target.as_deref(), r),
            SEARCH => return Ok(self.update_search(dt)),
            _ => {}
        }
        let Some(target) = target.filter(|_| self.target != -1) else {
            return self.update_without_target(c, unit, r);
        };
        match self.state {
            WALK if self.timer < 0.0 => self.update_walk_wait(c, unit, target, dt),
            WALK => self.update_walk(c, unit, me, target, r),
            FACE => self.update_face(c, unit, target, r),
            WORK => self.update_work(c, unit, target, r),
            _ => Err(GenieError::unsupported(
                "a Gather action in a state the slice does not model",
            )),
        }
    }

    /// Case 2, the search's wait: the timer counts up by the world's dt; at 0 it waits again
    /// ([`SEARCH_WAIT`]) and asks again ([`Next::Search`]).
    fn update_search(&mut self, dt: f32) -> Next {
        if self.timer < 0.0 {
            self.timer += dt;
            if 0.0 <= self.timer {
                self.timer = SEARCH_WAIT;
                return Next::Search;
            }
        }
        Next::Stay
    }

    /// Cases 3, 5 and 6 without a target: a load goes to its drop site ([`Next::Drop`]); a farmer
    /// waits for another farm (0xf3); else the search (`set_state(3)`). Any other state is not
    /// modelled.
    fn update_without_target(
        &mut self,
        c: &mut Step<'_>,
        unit: &mut Unit,
        r: &GatherReads,
    ) -> Result<Next, GenieError> {
        if !matches!(self.state, WALK | FACE | WORK) {
            return Err(GenieError::unsupported(
                "a Gather action in a state the slice does not model",
            ));
        }
        if 0.0 < unit.life.amount {
            return Ok(Next::Drop);
        }
        if waits_for_a_farm(r.master, self.target_type) {
            return self.begin_farm_wait(c, unit, r);
        }
        self.begin_search(c, unit, r)
    }

    /// Case 3's wait after a failed walk (the timer below 0 counts up by dt): at its end a
    /// gatherer of one stop walks to its point to search there (`set_state(11)`, within the
    /// target's radius, 0 for a farm of stops), one of a farm's stops to the next (4).
    fn update_walk_wait(
        &mut self,
        c: &mut Step<'_>,
        unit: &mut Unit,
        target: &Victim,
        dt: f32,
    ) -> Result<Next, GenieError> {
        self.timer += dt;
        if self.timer < 0.0 {
            return Ok(Next::Stay);
        }
        self.timer = 0.0;
        let mover = unit.body.mover_mut().ok_or(UNMOVING)?;
        if self.stops < 2 {
            let radius = if STOP_FARMS.contains(&target.master_id) {
                0.0
            } else {
                target.radius[0]
            };
            self.set_walk_back(radius, mover)?;
        } else {
            let rng = c.w.streams.get(Stream::Action);
            self.set_walk(target.position, target.radius, 0.0, mover, rng)?;
        }
        Ok(Next::Stay)
    }

    /// Case 5, the face: a farm not built yet is searched past (`set_state(3)`), a pasture short
    /// of its animals is not modelled, a farm another farmer works waits for another (0xf3; the
    /// search's notify is the caller's). Else the unit turns to its target (`FUN_141162db0`;
    /// turning over time it faces next update) and works (`set_state(7)`, its time at the stop
    /// 0) on a target holding something or one regathered; an empty one is not modelled.
    fn update_face(
        &mut self,
        c: &mut Step<'_>,
        unit: &mut Unit,
        target: &Victim,
        r: &GatherReads,
    ) -> Result<Next, GenieError> {
        if target.class == class::FARM {
            if target.state < 2 {
                return self.begin_search(c, unit, r);
            }
            if (target.master_id - 0x765) & !4 == 0 {
                return Err(GenieError::unsupported(
                    "a gather facing a pasture (its animals, slot 0x780)",
                ));
            }
            if 1 < target.workers {
                return self.begin_farm_wait(c, unit, r);
            }
        }
        let at = unit.body.position();
        let mover = unit.body.mover_mut().ok_or(UNMOVING)?;
        if !crate::combat::attack::face_target(mover, at, target.position, c.w.world_time)? {
            return Ok(Next::Stay);
        }
        if 0.0 < target.amount || REGATHERED.contains(&target.master_id) {
            self.set_work(c, unit, r);
            self.time_at_stop = 0.0;
            return Ok(Next::Stay);
        }
        Err(GenieError::unsupported(
            "a gather's target empty when faced (set_state 3)",
        ))
    }

    /// Case 6, the work: on a farm's stops (the game's `+0xc14` being 0) the time at the stop
    /// runs; past the time per stop with a stop left the farmer walks to the next (its count
    /// stays when the food is sold at once, attribute 96). Else it gathers
    /// ([`GatherAction::gather`]).
    fn update_work(
        &mut self,
        c: &mut Step<'_>,
        unit: &mut Unit,
        target: &mut Victim,
        r: &GatherReads,
    ) -> Result<Next, GenieError> {
        let dt = seconds(c.dt);
        if 1 < self.stops {
            self.time_at_stop += dt;
            if self.time_per_stop <= self.time_at_stop
                && u32::from(self.current_stop) + 1 < u32::from(self.stops)
            {
                if r.food_sold <= 0.0 {
                    self.current_stop += 1;
                }
                let mover = unit.body.mover_mut().ok_or(UNMOVING)?;
                let rng = c.w.streams.get(Stream::Action);
                self.set_walk(target.position, target.radius, 0.0, mover, rng)?;
                return Ok(Next::Stay);
            }
        }
        self.gather(c, unit, target, r)
    }

    /// State 4's `MoveTo` (case 3): it starts, walks, waits after the motion step gave up (a
    /// villager once) and ends: its update in state 1 returns 1 (the list deletes it and its
    /// destructor stops the unit), then `set_state(6)`. A failed `MoveTo` (code 3) near a target
    /// of more than 3 workers searches, else waits a random while.
    pub(super) fn update_walk(
        &mut self,
        c: &mut Step<'_>,
        unit: &mut Unit,
        me: Me,
        target: &Victim,
        r: &GatherReads,
    ) -> Result<Next, GenieError> {
        match self.move_to {
            Some(MoveTo::Start | MoveTo::Moving | MoveTo::Waiting) => {
                self.step_move(c, unit, me, r).map(|()| Next::Stay)
            }
            Some(MoveTo::Arrived) => {
                self.move_to = None;
                self.walk_to = None;
                unit.body.mover_mut().ok_or(UNMOVING)?.stop()?;
                self.set_face(c, unit, target, r)
            }
            // code 3: near a target with more than 3 workers it searches (set_state 3), else it
            // waits a random while (LAB_140a8ef80)
            Some(MoveTo::Failed) => {
                let mover = unit.body.mover().ok_or(UNMOVING)?;
                let me = [mover.stats.radius[0], mover.stats.radius[1]];
                let at = unit.body.position();
                let gap =
                    crate::movement::footprint_gap_squared(at, me, target.position, target.radius);
                if target.workers as f32 <= WAIT_WORKERS || WAIT_GAP <= gap {
                    self.random_wait(c, unit);
                    return Ok(Next::Stay);
                }
                self.begin_search(c, unit, r)
            }
            None => Err(GenieError::unsupported(
                "a Gather in state 4 without its MoveTo",
            )),
        }
    }

    /// A walking `MoveTo`'s update, its state 3, 4 or 2 (`FUN_140a992b0` on the sub-actions):
    /// it starts (a target's or a point's, within [`GatherAction::walk_range`]), walks, waits
    /// after the motion step gave up (a villager once); a `MoveTo` to the target asks again
    /// when the target left its point. The units in the unit's way go to its unit AI at once
    /// ([`Unit::post_in_the_way`](crate::objects::unit::Unit::post_in_the_way)).
    ///
    /// # Errors
    ///
    /// The walk's and the notifies'.
    pub(super) fn step_move(
        &mut self,
        c: &mut Step<'_>,
        unit: &mut Unit,
        mut me: Me,
        r: &GatherReads,
    ) -> Result<(), GenieError> {
        match self.move_to {
            Some(MoveTo::Start) => {
                if self.move_wait.state == 1 {
                    self.move_wait.state = 2;
                }
                self.move_to = Some(MoveTo::Moving);
                me.action_state = self.state;
                let range = self.walk_range;
                let goal = self.walk_to.map_or(Goal::Target(self.target), Goal::Point);
                let graphic = self.walk_sprite(unit, r);
                let (walk, sprite_rng, _) = c.walk_sprites_action(&mut unit.body, me);
                // the MoveTo's set_state(4) shows the walking sprite before the motion step
                let shown = Shown {
                    graphic,
                    sprite: &mut unit.sprite,
                    rng: sprite_rng,
                };
                let to = (goal, range, true);
                let walked = start_walk_shown(walk, to, PathAsk::default(), Some(shown))?;
                unit.post_in_the_way()?;
                match walked {
                    Some(state) => self.move_to = Some(state),
                    None => self.give_up(r),
                }
                Ok(())
            }
            Some(MoveTo::Moving) => {
                // a MoveTo to a point has no target to follow
                if self.walk_to.is_none()
                    && target_moved(&mut c.walk(&mut unit.body, me), self.target, true)?.again()
                {
                    self.move_to = Some(MoveTo::Start);
                    return Ok(());
                }
                me.action_state = self.state;
                let walked = walk_or_give_up(c.walk(&mut unit.body, me))?;
                unit.post_in_the_way()?;
                match walked {
                    Some(m) => self.move_to = Some(m),
                    None => self.give_up(r),
                }
                Ok(())
            }
            Some(MoveTo::Waiting) => {
                self.move_wait.timer += seconds(c.dt);
                if self.move_wait.delay <= self.move_wait.timer {
                    self.move_wait.timer = 0.0;
                    self.move_to = Some(MoveTo::Start);
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// The `MoveTo`'s answer to the motion step giving up ([`MoveWait::give_up`]; quiet). The
    /// computer player's test is not held: its villagers would fail at once.
    ///
    /// [`MoveWait::give_up`]: crate::movement::move_to::MoveWait::give_up
    pub(super) const fn give_up(&mut self, r: &GatherReads) {
        let waits = r.class == class::VILLAGER;
        self.move_to = Some(self.move_wait.give_up(waits, r.stance));
    }
}
