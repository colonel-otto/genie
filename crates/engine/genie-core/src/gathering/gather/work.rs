//! The face (state 6), the work (7), the drop-off's update (8) and a random wait.

use crate::GenieError;
use crate::combat::damage::Victim;
use crate::gathering::gather::{
    DROP_REACH, FACE, FARM_CLASS, FARM_FOOD_KIND, GatherAction, Gatherer, LAST_BIT, Next,
    REGATHERED, REMAINDER, SPECIAL_TARGETS, WAIT_SHIFT, WORK,
};
use crate::gathering::hunt::Dropped;
use crate::movement::MoveStats;
use crate::movement::move_to::{
    Goal, MoveTo, Walk, start_walk_or_give_up, target_moved, walk_or_give_up,
};
use crate::objects::sprite::new_sprite;

/// Food and the food kinds (0, 15 to 17): a gatherer keeps a load of one for another.
pub(super) const fn food(kind: i16) -> bool {
    kind == 0 || (15 <= kind && kind <= 17)
}

impl GatherAction {
    /// `set_state(6)` (case 6) on a target: a farm, masters 357 and 1188 and a target not yet
    /// built (state below 2, the unit's slot 0x380) are not modelled; the timer clears and the
    /// unit takes the walking sprite (`FUN_140a8b660`; slot 0x68 takes the standing one for
    /// none).
    pub(super) fn set_face(
        &mut self,
        target: &Victim,
        stats: Option<&mut MoveStats>,
        g: &mut Gatherer<'_>,
    ) -> Result<Next, GenieError> {
        self.state = FACE;
        // another player's farm is the gatherer's to take (FUN_141185360, FUN_1411858f0)
        if target.class == FARM_CLASS && target.player != g.player {
            return Err(GenieError::unsupported(
                "a gather's set_state 6 on another player's farm (FUN_141185360)",
            ));
        }
        if SPECIAL_TARGETS.contains(&target.master_id) {
            return Err(GenieError::unsupported(
                "a gather's set_state 6 on master 357 or 1188",
            ));
        }
        if target.state < 2 {
            return Ok(Next::Build);
        }
        if (target.master_id - 0x765) & !4 == 0 {
            return Err(GenieError::unsupported(
                "a gather's set_state 6 on master 1893 or 1897",
            ));
        }
        self.timer = 0.0;
        if let Some(s) = self.walk_sprite(g).or(g.sprites.standing) {
            new_sprite(g.sprite, stats, s, g.sprite_rng);
        }
        Ok(Next::Stay)
    }

    /// `set_state(7)` (case 7): the gathering starts (`+0x80`), the timer clears; without the
    /// game's farm setting the unit's place is `mGatherX`, `mGatherY`. A load of another resource
    /// is dropped unless both are food (`FUN_14119d310` with 0; then the unit keeps the kind it
    /// carries). The unit takes the working sprite (slot 0x68; the standing one for none); the
    /// gathering sound (slot 0x4c0) is not held.
    pub(super) fn set_work(
        &mut self,
        at: [f32; 3],
        stats: Option<&mut MoveStats>,
        g: &mut Gatherer<'_>,
    ) {
        self.state = WORK;
        self.worked = true;
        self.timer = 0.0;
        if !g.farm_stops {
            self.gather_at = [at[0], at[1]];
        }
        let kind = self.task.resource;
        if *g.held_kind != kind && !(food(kind) && food(*g.held_kind)) {
            *g.held_kind = kind;
            *g.held = 0.0;
        }
        if let Some(s) = g.sprites.working.or(g.sprites.standing) {
            new_sprite(g.sprite, stats, s, g.sprite_rng);
        }
    }

    /// State 7 (case 6) with one stop: the work, `(work rate) × work_val1 × dt`, at most what the
    /// target holds, times the productivity. A load that would pass the capacity ends the
    /// gathering ([`Next::Drop`]); else the target loses the work, its last bit (above 0, below
    /// 0.1) going with a thousandth of its master's first storage amount, the unit gains, and a
    /// remainder below 0.1 of the capacity rounds the load up. A farm's food under the player's
    /// attribute 96 is sold at once ([`Gatherer::sold`]); else a full load, or an empty target
    /// that cannot be gathered empty (the combat object's slot 0x2b8), ends it. The game's
    /// `+0xc14` (`FUN_140c884c0`) is 0, as the tick oracle holds it.
    pub(super) fn gather(&self, target: &mut Victim, g: &mut Gatherer<'_>, dt: f32) -> Next {
        let work = (g.work_rate * self.task.work_value) * dt;
        let take = if work <= target.amount {
            work
        } else {
            target.amount
        };
        let capacity = g.capacity;
        let mut gain = g.productivity * take;
        if capacity < gain + *g.held {
            return Next::Drop;
        }
        target.amount += -take;
        // the last bit and the storage's share summed first (`0x140a8f751`)
        if target.amount < REMAINDER && 0.0 < target.amount {
            gain += target.storage * LAST_BIT + target.amount;
            target.amount = 0.0;
        }
        *g.held += gain;
        let over = (*g.held - capacity).abs();
        if *g.held < capacity && over < REMAINDER && over <= target.amount {
            target.amount += -over;
            *g.held += over;
        }
        // a farm's food sold at once (attribute 96): the load goes to the player as food (slot
        // 0xf8 with 1, the caller's), the unit holds none, and the gathering goes on
        if 0.0 < g.food_sold && target.class == FARM_CLASS && *g.held_kind == FARM_FOOD_KIND {
            g.sold += *g.held;
            *g.held = 0.0;
            return Next::Stay;
        }
        let empty = target.amount <= 0.0 && !REGATHERED.contains(&target.master_id);
        if over < REMAINDER || empty {
            Next::Drop
        } else {
            Next::Stay
        }
    }

    /// State 8's update (case 7): the `MoveTo` to the drop site walks; once it ended (its update
    /// in state 1 returns 1: the list deletes it and its destructor stops the unit) the load goes
    /// to the player at a drop site that takes it (`site`) within `DROP_REACH` (squared): the
    /// resource of what the unit carries ([`crate::gathering::hunt::resource_kind`]), which the caller adds;
    /// the unit carries nothing after (`FUN_14119d310` with −1). Returns the resource and the
    /// amount; the caller lets the drop site go and walks back to the target (`set_state(4)`).
    /// Not modelled: the timer's wait, a `MoveTo` that waits or failed (codes 3 to 5), a site that
    /// does not take the load (`set_state` 2) or is out of reach (the wait).
    ///
    /// # Errors
    ///
    /// What the drop-off does not model.
    pub fn update_drop(
        &mut self,
        mut walk: Walk<'_>,
        g: &mut Gatherer<'_>,
        attributes: usize,
        site: Option<crate::gathering::hunt::DropOff>,
    ) -> Result<Dropped, GenieError> {
        if self.timer < 0.0 {
            self.timer += walk.dt;
            if self.timer < 0.0 {
                return Ok(Dropped::Going);
            }
            self.timer = 0.0;
            return Ok(Dropped::Again);
        }
        match self.move_to {
            Some(MoveTo::Start) => {
                self.move_to = Some(MoveTo::Moving);
                walk.me.action_state = self.state;
                let range = self.task.range;
                let walked = start_walk_or_give_up(
                    walk.reborrow(),
                    Goal::Target(self.carry_to),
                    range,
                    true,
                )?;
                if walked.is_none_or(|w| w == MoveTo::Moving)
                    && let Some(s) = self.walk_sprite(g)
                {
                    new_sprite(g.sprite, walk.body.move_stats_mut(), s, g.sprite_rng);
                }
                match walked {
                    Some(state) => self.move_to = Some(state),
                    None => self.give_up(g),
                }
                return Ok(Dropped::Going);
            }
            Some(MoveTo::Moving) => {
                if target_moved(&walk, self.carry_to, true)?.again() {
                    self.move_to = Some(MoveTo::Start);
                    return Ok(Dropped::Going);
                }
                walk.me.action_state = self.state;
                let walked = walk_or_give_up(walk.reborrow())?;
                self.post_in_the_way(&mut walk, g);
                match walked {
                    Some(m) => self.move_to = Some(m),
                    None => self.give_up(g),
                }
                return Ok(Dropped::Going);
            }
            Some(MoveTo::Waiting) => {
                self.move_wait.timer += walk.dt;
                if self.move_wait.delay <= self.move_wait.timer {
                    self.move_wait.timer = 0.0;
                    self.move_to = Some(MoveTo::Start);
                }
                return Ok(Dropped::Going);
            }
            // codes 3 to 5: a wait of a random share of a second less a half (the Action
            // stream's GetRandFloat0to1), the unit's animation stopped (FUN_141161850)
            Some(MoveTo::Failed) => {
                self.random_wait(g);
                return Ok(Dropped::Going);
            }
            Some(MoveTo::Arrived) => {
                self.move_to = None;
                walk.body
                    .mover_mut()
                    .ok_or(GenieError::unsupported("a gatherer without movement state"))?
                    .stop()?;
            }
            None => {
                return Err(GenieError::unsupported(
                    "a Gather in state 8 without its MoveTo",
                ));
            }
        }
        // the drop site gone: set_state(8) again
        let Some(site) = site else {
            return Ok(Dropped::Again);
        };
        if !site.takes {
            self.set_idle(walk.body.move_stats_mut(), g);
            return Ok(Dropped::Idle);
        }
        if DROP_REACH < site.gap_squared {
            self.random_wait(g);
            return Ok(Dropped::Going);
        }
        let kind = crate::gathering::hunt::resource_kind(*g.held_kind, attributes);
        let amount = *g.held;
        *g.held_kind = -1;
        *g.held = 0.0;
        Ok(Dropped::Load(kind, amount))
    }

    /// `LAB_140a8ef80`: the timer takes a draw of the Action stream (`GetRandFloat0to1`) less a
    /// half, and the unit's animation stops (`FUN_141161850`, the caller's).
    pub(super) fn random_wait(&mut self, g: &mut Gatherer<'_>) {
        self.timer = g.action_rng.float01() - WAIT_SHIFT;
        g.anim_stopped = true;
    }
}
