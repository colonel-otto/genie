//! The face (state 6), the work (7), the drop-off's update (8) and a random wait.

use crate::GenieError;
use crate::combat::damage::Victim;
use crate::gathering::gather::update::UNMOVING;
use crate::gathering::gather::{
    DROP_REACH, FACE, GatherAction, GatherReads, LAST_BIT, Next, REGATHERED, REMAINDER,
    SPECIAL_TARGETS, WAIT_SHIFT, WORK,
};
use crate::gathering::hunt::Dropped;
use crate::gathering::make::FARM_FOOD_KIND;
use crate::math::rng::Stream;
use crate::movement::move_to::TargetMoved;
use crate::movement::move_to::walk_or_give_up;
use crate::movement::move_to::{Goal, MoveTo, Shown, start_walk_shown, target_moved};
use crate::movement::{Me, PathAsk};
use crate::objects::lifecycle::seconds;
use crate::objects::masters::class;
use crate::objects::sprite::{Graphic, new_sprite};
use crate::objects::unit::Unit;
use crate::world::step::Step;

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
        c: &mut Step<'_>,
        unit: &mut Unit,
        target: &Victim,
        r: &GatherReads,
    ) -> Result<Next, GenieError> {
        self.state = FACE;
        // another player's farm is the gatherer's to take (FUN_141185360, FUN_1411858f0)
        if target.class == class::FARM && target.player != c.player {
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
        Self::show(c, unit, self.walk_sprite(unit, r).or(r.sprites.standing));
        Ok(Next::Stay)
    }

    /// Slot 0x68 with `sprite` (`FUN_141161280`, [`new_sprite`]) on the unit at once: its new
    /// sprite draws from the Sprite stream; none shows nothing new.
    pub(super) fn show(c: &mut Step<'_>, unit: &mut Unit, sprite: Option<Graphic>) {
        if let Some(s) = sprite {
            let rng = c.w.streams.get(Stream::Sprite);
            new_sprite(&mut unit.sprite, unit.body.move_stats_mut(), s, rng);
        }
    }

    /// `set_state(7)` (case 7): the gathering starts (`+0x80`), the timer clears; without the
    /// game's farm setting the unit's place is `mGatherX`, `mGatherY`. A load of another resource
    /// is dropped unless both are food (`FUN_14119d310` with 0; then the unit keeps the kind it
    /// carries). The unit takes the working sprite (slot 0x68; the standing one for none); the
    /// gathering sound (slot 0x4c0) is not held.
    pub(super) fn set_work(&mut self, c: &mut Step<'_>, unit: &mut Unit, r: &GatherReads) {
        self.state = WORK;
        self.worked = true;
        self.timer = 0.0;
        if !r.farm_stops {
            let at = unit.body.position();
            self.gather_at = [at[0], at[1]];
        }
        let kind = self.task.resource;
        let held = unit.life.attribute_type;
        if held != kind && !(food(kind) && food(held)) {
            unit.life.attribute_type = kind;
            unit.life.amount = 0.0;
        }
        Self::show(c, unit, r.sprites.working.or(r.sprites.standing));
    }

    /// State 7 (case 6) with one stop: the work, `(work rate) × work_val1 × dt`, at most what the
    /// target holds, times the productivity. A load that would pass the capacity ends the
    /// gathering ([`Next::Drop`]); else the target loses the work, its last bit (above 0, below
    /// 0.1) going with a thousandth of its master's first storage amount, the unit gains, and a
    /// remainder below 0.1 of the capacity rounds the load up. A farm's food under the player's
    /// attribute 96 is sold at once: the player gains it as food (slot 0xf8 with 1,
    /// [`Step::give_load`]); else a full load, or an empty target that cannot be gathered empty
    /// (the combat object's slot 0x2b8), ends it. The game's `+0xc14` (`FUN_140c884c0`) is 0,
    /// as the tick oracle holds it.
    ///
    /// # Errors
    ///
    /// The sale's.
    pub(super) fn gather(
        &self,
        c: &mut Step<'_>,
        unit: &mut Unit,
        target: &mut Victim,
        r: &GatherReads,
    ) -> Result<Next, GenieError> {
        let held = &mut unit.life.amount;
        let work = (r.work_rate * self.task.work_value) * seconds(c.dt);
        let take = if work <= target.amount {
            work
        } else {
            target.amount
        };
        let capacity = r.capacity;
        let mut gain = r.productivity * take;
        if capacity < gain + *held {
            return Ok(Next::Drop);
        }
        target.amount += -take;
        // the last bit and the storage's share summed first (`0x140a8f751`)
        if target.amount < REMAINDER && 0.0 < target.amount {
            gain += target.storage * LAST_BIT + target.amount;
            target.amount = 0.0;
        }
        *held += gain;
        let over = (*held - capacity).abs();
        if *held < capacity && over < REMAINDER && over <= target.amount {
            target.amount += -over;
            *held += over;
        }
        // a farm's food sold at once (attribute 96): the load goes to the player as food (slot
        // 0xf8 with 1), the unit holds none, and the gathering goes on
        if 0.0 < r.food_sold
            && target.class == class::FARM
            // The food kind a farm holds (`+0x308` 16): the food sold at once.
            && unit.life.attribute_type == FARM_FOOD_KIND
        {
            let sold = std::mem::take(&mut unit.life.amount);
            c.give_load(0, sold)?;
            return Ok(Next::Stay);
        }
        let empty = target.amount <= 0.0 && !REGATHERED.contains(&target.master_id);
        Ok(if over < REMAINDER || empty {
            Next::Drop
        } else {
            Next::Stay
        })
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
    pub(crate) fn update_drop(
        &mut self,
        c: &mut Step<'_>,
        unit: &mut Unit,
        mut me: Me,
        r: &GatherReads,
        site: Option<crate::gathering::hunt::DropOff>,
    ) -> Result<Dropped, GenieError> {
        if self.timer < 0.0 {
            self.timer += seconds(c.dt);
            if self.timer < 0.0 {
                return Ok(Dropped::Going);
            }
            self.timer = 0.0;
            return Ok(Dropped::Again);
        }
        match self.move_to {
            Some(MoveTo::Start) => {
                self.move_to = Some(MoveTo::Moving);
                me.action_state = self.state;
                let range = self.task.range;
                let graphic = self.walk_sprite(unit, r);
                let (walk, sprite_rng, _) = c.walk_sprites_action(&mut unit.body, me);
                // the MoveTo's set_state(4) shows the walking sprite before the motion step
                let shown = Shown {
                    graphic,
                    sprite: &mut unit.sprite,
                    rng: sprite_rng,
                };
                let to = (Goal::Target(self.carry_to), range, true);
                let walked = start_walk_shown(walk, to, PathAsk::default(), Some(shown))?;
                match walked {
                    Some(state) => self.move_to = Some(state),
                    None => self.give_up(r),
                }
                return Ok(Dropped::Going);
            }
            Some(MoveTo::Moving) => {
                if target_moved(
                    &mut c.walk(&mut unit.body, me),
                    self.carry_to,
                    TargetMoved::AsksAgain,
                )?
                .again()
                {
                    self.move_to = Some(MoveTo::Start);
                    return Ok(Dropped::Going);
                }
                me.action_state = self.state;
                let walked = walk_or_give_up(c.walk(&mut unit.body, me))?;
                unit.post_in_the_way()?;
                match walked {
                    Some(m) => self.move_to = Some(m),
                    None => self.give_up(r),
                }
                return Ok(Dropped::Going);
            }
            Some(MoveTo::Waiting) => {
                self.move_wait.timer += seconds(c.dt);
                if self.move_wait.delay <= self.move_wait.timer {
                    self.move_wait.timer = 0.0;
                    self.move_to = Some(MoveTo::Start);
                }
                return Ok(Dropped::Going);
            }
            // codes 3 to 5: a wait of a random share of a second less a half (the Action
            // stream's GetRandFloat0to1), the unit's animation stopped (FUN_141161850)
            Some(MoveTo::Failed) => {
                self.random_wait(c, unit);
                return Ok(Dropped::Going);
            }
            Some(MoveTo::Arrived) => {
                self.move_to = None;
                unit.body.mover_mut().ok_or(UNMOVING)?.stop()?;
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
            self.set_idle(c, unit, r);
            return Ok(Dropped::Idle);
        }
        if DROP_REACH < site.gap_squared {
            self.random_wait(c, unit);
            return Ok(Dropped::Going);
        }
        let attributes = c.w.players[c.player].attributes.len();
        let kind = crate::gathering::hunt::resource_kind(unit.life.attribute_type, attributes);
        let amount = unit.life.amount;
        unit.life.attribute_type = -1;
        unit.life.amount = 0.0;
        Ok(Dropped::Load(kind, amount))
    }

    /// `LAB_140a8ef80`: the timer takes a draw of the Action stream (`GetRandFloat0to1`) less a
    /// half, and the unit's animation stops at once (`FUN_141161850`: its `+0x7b8` 0).
    pub(super) fn random_wait(&mut self, c: &mut Step<'_>, unit: &mut Unit) {
        self.timer = c.w.streams.get(Stream::Action).float01() - WAIT_SHIFT;
        unit.body.set_anim_speed(0.0);
    }
}
