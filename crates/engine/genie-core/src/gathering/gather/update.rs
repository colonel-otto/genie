//! One update (slot 6, `0x140a8e1d0`) in states 4, 6 and 7: the walk's `MoveTo`, the units
//! in the way and the teardown.

use crate::GenieError;
use crate::combat::attack::set_sprite;
use crate::combat::damage::Victim;
use crate::gathering::gather::{
    DONE, FACE, FARM_CLASS, FARM_WAIT, GatherAction, Gatherer, IDLE, MOVED, Next, REGATHERED,
    RESEED_WALK, SEARCH, SEARCH_WAIT, STOP_FARMS, StopRate, VILLAGER_CLASS, WAIT_GAP, WAIT_WORKERS,
    WALK, WALK_BACK, WORK, waits_for_a_farm,
};
use crate::gathering::hunt::walk_sprite;
use crate::movement::Mover;
use crate::movement::move_to::{
    Goal, MoveTo, Walk, start_walk_or_give_up, target_moved, walk_or_give_up,
};

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
        g: &Gatherer<'_>,
    ) -> Option<crate::objects::sprite::Graphic> {
        walk_sprite(self.task.resource, *g.held_kind, *g.held, &g.sprites)
    }

    /// One update (slot 6, `0x140a8e1d0`) in states 4, 6 and 7, `target` the target the caller
    /// holds (None once it went). State 4 walks the `MoveTo` (`FUN_140a992b0`); its end (codes 1
    /// and 2) is `set_state(6)`. State 6 faces the target (`FUN_141162db0`) and, while it holds
    /// something, `set_state(7)`. State 7 gathers ([`GatherAction::gather`]). Without a target a
    /// loaded gatherer takes its load off ([`Next::Drop`]).
    ///
    /// # Errors
    ///
    /// What the update does not model.
    pub fn update(
        &mut self,
        walk: Walk<'_>,
        target: Option<&mut Victim>,
        g: &mut Gatherer<'_>,
        dt: f32,
    ) -> Result<Next, GenieError> {
        // a farm's time per stop follows the gatherer's work rate, every update
        if target
            .as_deref()
            .is_some_and(|t| STOP_FARMS.contains(&t.master_id))
        {
            self.time_per_stop = StopRate {
                work_rate: g.work_rate,
                capacity: g.capacity,
                productivity: g.stop_productivity,
            }
            .time_per_stop(self.task.work_value, self.stops);
        }
        match self.state {
            DONE => return Ok(Next::Done),
            IDLE => return Ok(Next::Stay),
            MOVED => return self.update_moved(walk, g),
            WALK_BACK => return self.update_walk_back(walk, g),
            FARM_WAIT => return self.update_farm_wait(walk, target.as_deref(), g, dt),
            RESEED_WALK => return self.update_reseed_walk(walk, target.as_deref(), g),
            SEARCH => return Ok(self.update_search(dt)),
            _ => {}
        }
        let Some(target) = target.filter(|_| self.target != -1) else {
            return self.update_without_target(walk, g);
        };
        match self.state {
            WALK if self.timer < 0.0 => self.update_walk_wait(walk, target, g, dt),
            WALK => self.update_walk(walk, target, g),
            FACE => self.update_face(walk, target, g),
            WORK => self.update_work(walk, target, g, dt),
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
        mut walk: Walk<'_>,
        g: &mut Gatherer<'_>,
    ) -> Result<Next, GenieError> {
        if !matches!(self.state, WALK | FACE | WORK) {
            return Err(GenieError::unsupported(
                "a Gather action in a state the slice does not model",
            ));
        }
        if 0.0 < *g.held {
            return Ok(Next::Drop);
        }
        if waits_for_a_farm(g.master, self.target_type) {
            return self.begin_farm_wait(walk.mover.as_deref_mut(), g);
        }
        self.begin_search(walk.mover.as_deref_mut(), g)
    }

    /// Case 3's wait after a failed walk (the timer below 0 counts up by dt): at its end a
    /// gatherer of one stop walks to its point to search there (`set_state(11)`, within the
    /// target's radius, 0 for a farm of stops), one of a farm's stops to the next (4).
    fn update_walk_wait(
        &mut self,
        mut walk: Walk<'_>,
        target: &Victim,
        g: &mut Gatherer<'_>,
        dt: f32,
    ) -> Result<Next, GenieError> {
        self.timer += dt;
        if self.timer < 0.0 {
            return Ok(Next::Stay);
        }
        self.timer = 0.0;
        let mover = walk
            .mover
            .as_deref_mut()
            .ok_or(GenieError::unsupported("a gatherer without movement state"))?;
        if self.stops < 2 {
            let radius = if STOP_FARMS.contains(&target.master_id) {
                0.0
            } else {
                target.radius[0]
            };
            self.set_walk_back(radius, mover)?;
        } else {
            self.set_walk(target.position, target.radius, 0.0, mover, g.action_rng)?;
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
        mut walk: Walk<'_>,
        target: &Victim,
        g: &mut Gatherer<'_>,
    ) -> Result<Next, GenieError> {
        if target.class == FARM_CLASS {
            if target.state < 2 {
                return self.begin_search(walk.mover.as_deref_mut(), g);
            }
            if (target.master_id - 0x765) & !4 == 0 {
                return Err(GenieError::unsupported(
                    "a gather facing a pasture (its animals, slot 0x780)",
                ));
            }
            if 1 < target.workers {
                return self.begin_farm_wait(walk.mover.as_deref_mut(), g);
            }
        }
        let at = walk
            .world
            .get(walk.me.id)
            .map_or(walk.me.position, |p| p.position);
        let mover = walk
            .mover
            .as_deref_mut()
            .ok_or(GenieError::unsupported("a gatherer without movement state"))?;
        if !crate::combat::attack::face_target(mover, at, target.position, g.world_time)? {
            return Ok(Next::Stay);
        }
        if 0.0 < target.amount || REGATHERED.contains(&target.master_id) {
            self.set_work(walk.me.position, g);
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
        mut walk: Walk<'_>,
        target: &mut Victim,
        g: &mut Gatherer<'_>,
        dt: f32,
    ) -> Result<Next, GenieError> {
        if 1 < self.stops {
            self.time_at_stop += dt;
            if self.time_per_stop <= self.time_at_stop
                && u32::from(self.current_stop) + 1 < u32::from(self.stops)
            {
                if g.food_sold <= 0.0 {
                    self.current_stop += 1;
                }
                let mover = walk
                    .mover
                    .as_deref_mut()
                    .ok_or(GenieError::unsupported("a gatherer without movement state"))?;
                self.set_walk(target.position, target.radius, 0.0, mover, g.action_rng)?;
                return Ok(Next::Stay);
            }
        }
        Ok(self.gather(target, g, dt))
    }

    /// State 4's `MoveTo` (case 3): it starts, walks, waits after the motion step gave up (a
    /// villager once) and ends: its update in state 1 returns 1 (the list deletes it and its
    /// destructor stops the unit), then `set_state(6)`. A failed `MoveTo` (code 3) is not
    /// modelled.
    pub(super) fn update_walk(
        &mut self,
        mut walk: Walk<'_>,
        target: &Victim,
        g: &mut Gatherer<'_>,
    ) -> Result<Next, GenieError> {
        match self.move_to {
            Some(MoveTo::Start | MoveTo::Moving | MoveTo::Waiting) => {
                self.step_move(walk, g).map(|()| Next::Stay)
            }
            Some(MoveTo::Arrived) => {
                self.move_to = None;
                self.walk_to = None;
                walk.mover
                    .as_deref_mut()
                    .ok_or(GenieError::unsupported("a gatherer without movement state"))?
                    .stop()?;
                self.set_face(target, g)
            }
            // code 3: near a target with more than 3 workers it searches (set_state 3), else it
            // waits a random while (LAB_140a8ef80)
            Some(MoveTo::Failed) => {
                let mover = walk
                    .mover
                    .as_deref()
                    .ok_or(GenieError::unsupported("a gatherer without movement state"))?;
                let me = [mover.stats.radius[0], mover.stats.radius[1]];
                let at = walk
                    .world
                    .get(walk.me.id)
                    .map_or(walk.me.position, |p| p.position);
                let gap =
                    crate::movement::footprint_gap_squared(at, me, target.position, target.radius);
                if target.workers as f32 <= WAIT_WORKERS || WAIT_GAP <= gap {
                    self.random_wait(g);
                    return Ok(Next::Stay);
                }
                self.begin_search(walk.mover.as_deref_mut(), g)
            }
            None => Err(GenieError::unsupported(
                "a Gather in state 4 without its MoveTo",
            )),
        }
    }

    /// A walking `MoveTo`'s update, its state 3, 4 or 2 (`FUN_140a992b0` on the sub-actions):
    /// it starts (a target's or a point's, within [`GatherAction::walk_range`]), walks, waits
    /// after the motion step gave up (a villager once); a `MoveTo` to the target asks again
    /// when the target left its point.
    pub(super) fn step_move(
        &mut self,
        mut walk: Walk<'_>,
        g: &mut Gatherer<'_>,
    ) -> Result<(), GenieError> {
        match self.move_to {
            Some(MoveTo::Start) => {
                if self.move_wait.state == 1 {
                    self.move_wait.state = 2;
                }
                self.move_to = Some(MoveTo::Moving);
                walk.me.action_state = self.state;
                let range = self.walk_range;
                let goal = self.walk_to.map_or(Goal::Target(self.target), Goal::Point);
                let walked = start_walk_or_give_up(walk.reborrow(), goal, range, true)?;
                self.post_in_the_way(&mut walk, g);
                // the MoveTo's set_state(4) shows the walking sprite
                if walked.is_none_or(|w| w == MoveTo::Moving)
                    && let Some(s) = self.walk_sprite(g)
                {
                    set_sprite(g.sprite, s, g.sprite_rng);
                }
                match walked {
                    Some(state) => self.move_to = Some(state),
                    None => self.give_up(g),
                }
                Ok(())
            }
            Some(MoveTo::Moving) => {
                // a MoveTo to a point has no target to follow
                if self.walk_to.is_none() && target_moved(&walk, self.target, true)?.again() {
                    self.move_to = Some(MoveTo::Start);
                    return Ok(());
                }
                walk.me.action_state = self.state;
                let walked = walk_or_give_up(walk.reborrow())?;
                self.post_in_the_way(&mut walk, g);
                match walked {
                    Some(m) => self.move_to = Some(m),
                    None => self.give_up(g),
                }
                Ok(())
            }
            Some(MoveTo::Waiting) => {
                self.move_wait.timer += walk.dt;
                if self.move_wait.delay <= self.move_wait.timer {
                    self.move_wait.timer = 0.0;
                    self.move_to = Some(MoveTo::Start);
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// The units in the way the motion step told the unit AI of ([`crate::unit_ai::IN_THE_WAY`]).
    pub(super) fn post_in_the_way(&self, walk: &mut Walk<'_>, g: &mut Gatherer<'_>) {
        if let Some(m) = walk.mover.as_deref_mut() {
            g.posted.extend(
                m.in_the_way
                    .drain(..)
                    .map(|id| (crate::unit_ai::IN_THE_WAY, [id, -1, -1])),
            );
        }
    }

    /// The `MoveTo`'s answer to the motion step giving up ([`MoveWait::give_up`]; quiet). The
    /// computer player's test is not held: its villagers would fail at once.
    ///
    /// [`MoveWait::give_up`]: crate::movement::move_to::MoveWait::give_up
    pub(super) const fn give_up(&mut self, g: &Gatherer<'_>) {
        let waits = g.class == VILLAGER_CLASS;
        self.move_to = Some(self.move_wait.give_up(waits, g.stance));
    }
}
