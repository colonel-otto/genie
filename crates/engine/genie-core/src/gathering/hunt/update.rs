//! One update (slot 6, `0x140ac7a00`), a target lost on the way and the attack on live
//! livestock (state 12).

use crate::GenieError;
use crate::combat::attack::{Attack, AttackReads, Fight, set_walker_anim};
use crate::gathering::hunt::{
    ATTACK, FACE, GATHER, Herd, HuntAction, HuntNext, HuntReads, LEAD, PREDATOR, SEARCH, Shown,
    TREE_CLASS, WALK, WALK_SEARCH, walk_sprite,
};
use crate::math::rng::Stream;
use crate::movement::move_to::{Goal, MoveTo, start_walk_shown, target_moved, walk_or_give_up};
use crate::movement::{Me, Mover, PathAsk};
use crate::objects::lifecycle::seconds;
use crate::objects::unit::Unit;
use crate::world::step::Step;

impl HuntAction {
    /// One update (slot 6, `0x140ac7a00`) of `unit`'s hunt on the step, the hunt copied out of
    /// its list: state 12 updates the Attack (`FUN_140a992b0`); its end on the dead target (code
    /// 5) runs `FUN_140ac44b0` and `set_state(4)`. State 4 updates the `MoveTo`; its end (code 1)
    /// runs `FUN_140ac44b0` and `set_state(6)`. State 6 faces the target (`FUN_141162db0`) and,
    /// while it holds something, `set_state(7)`; state 7 gathers. Its notifies, the target it
    /// lets go and the target's changes reach the world at once; what follows that reads the
    /// hunt in its list is the caller's ([`HuntNext`]). A computer player's hunt and the other
    /// codes are not modelled.
    ///
    /// # Errors
    ///
    /// What the update does not model.
    pub(crate) fn update(
        &mut self,
        c: &mut Step<'_>,
        unit: &mut Unit,
        me: Me,
        f: &mut Fight,
        r: &HuntReads,
    ) -> Result<HuntNext, GenieError> {
        if r.attack.computer {
            return Err(GenieError::unsupported(
                "a computer player's hunt (FUN_140fde4f0)",
            ));
        }
        // a target gone or past state 6 went at the update's start (slot 0x68 with none, the
        // caller's): in state 4 its `MoveTo`, quiet, forgets it too and returns 5
        if self.state == WALK && self.target == -1 && 0.0 <= self.timer && self.move_to.is_some() {
            let mover = unit.body.mover_mut().ok_or(crate::world::HUNTER_UNMOVING)?;
            self.lost_on_the_way(mover, r)?;
            return Ok(HuntNext::Stay);
        }
        match self.state {
            ATTACK => self.update_attack(c, unit, me, f, r),
            // case 3: a hunt that gave up on its MoveTo waits (its timer below 0), then set_state 11
            WALK if self.timer < 0.0 => {
                let mover = unit.body.mover_mut().ok_or(crate::world::HUNTER_UNMOVING)?;
                self.wait_then_walk(seconds(c.dt), mover)?;
                Ok(HuntNext::Stay)
            }
            WALK => self.update_walk(c, unit, me, f, r),
            LEAD => self.update_lead(c, unit, f, r),
            FACE => self.update_face(c, unit, f, r),
            // case 6 without a target (or a task): set_state(8) to carry the load off, set_state(3)
            // with none
            GATHER if self.target == -1 => Ok(if self.ends_without_target(unit.life.amount) {
                HuntNext::Drop
            } else {
                HuntNext::Search
            }),
            GATHER => {
                let full = self.gather(seconds(c.dt), unit, &mut f.victim, r)?;
                write_target_back(c, f)?;
                Ok(if full { HuntNext::Drop } else { HuntNext::Stay })
            }
            _ => Err(GenieError::unsupported(
                "a Hunt action in a state the slice does not model",
            )),
        }
    }

    /// State 12, the Attack's update (`FUN_140a992b0`) and its end: ended (codes 1, 2 and 5)
    /// with no target left the hunt searches (`set_state(3)`); a predator (class 10) of a unit AI
    /// with orders queued (`FUN_140c564a0` above 0) would end the hunt (`set_state(1)`) under
    /// the game's setting `+0x98`, not modelled; else the decay check (`FUN_140ac44b0`) and the
    /// walk to the target (`set_state(4)`). Failed (3, its `MoveTo` did) the caller's search for
    /// the next tree follows, else `set_state(3)`; the target gone inside another (code 4,
    /// `LAB_140ac7ecd`) is let go at once and the hunt walks to its point to search there
    /// (`set_state(11)`).
    fn update_attack(
        &mut self,
        c: &mut Step<'_>,
        unit: &mut Unit,
        me: Me,
        f: &mut Fight,
        r: &HuntReads,
    ) -> Result<HuntNext, GenieError> {
        let attack = self.attack.as_mut().ok_or(GenieError::unsupported(
            "a Hunt in state 12 without its Attack",
        ))?;
        match attack.update(c, unit, me, f, &r.attack)? {
            0 | 6.. => Ok(HuntNext::Stay),
            1 | 2 | 5 if self.target == -1 => {
                self.tear_down(unit.body.mover_mut())?;
                self.state = SEARCH;
                Ok(HuntNext::Search)
            }
            1 | 2 | 5
                if r.predator_hunts_end && f.victim.class == PREDATOR && !r.attack.queue_empty =>
            {
                Err(GenieError::unsupported(
                    "a hunt that ends with its Attack on a predator (set_state 1)",
                ))
            }
            1 | 2 | 5 => {
                self.decay_check(c, r.stops_decay, f)?;
                self.set_walk(&c.w.spatial, &mut unit.body, &f.victim, f.own.player)?;
                Ok(HuntNext::Stay)
            }
            3 => Ok(HuntNext::AttackFailed),
            _ => {
                let target = self.let_go();
                unit.hunt_release(c, [target])?;
                let mover = unit.body.mover_mut().ok_or(crate::world::HUNTER_UNMOVING)?;
                self.set_point_walk(WALK_SEARCH, mover)?;
                Ok(HuntNext::Stay)
            }
        }
    }

    /// State 4, the walk's `MoveTo` (`FUN_140a992b0`): it starts ([`HuntAction::walk_start`]),
    /// walks ([`HuntAction::walk_moving`]), fails ([`HuntAction::walk_failed`]), waits (case 2:
    /// the timer runs by dt; at its end it asks again, `set_state(3)`) or arrives
    /// ([`HuntAction::walk_arrived`]).
    fn update_walk(
        &mut self,
        c: &mut Step<'_>,
        unit: &mut Unit,
        me: Me,
        f: &mut Fight,
        r: &HuntReads,
    ) -> Result<HuntNext, GenieError> {
        match self.move_to {
            Some(MoveTo::Start) => self.walk_start(c, unit, me, f, r),
            Some(MoveTo::Moving) => self.walk_moving(c, unit, me, f, &r.attack),
            Some(MoveTo::Failed) => self.walk_failed(unit, r),
            Some(MoveTo::Waiting) => {
                self.move_wait.timer += seconds(c.dt);
                if self.move_wait.delay <= self.move_wait.timer {
                    self.move_wait.timer = 0.0;
                    self.move_to = Some(MoveTo::Start);
                }
                return Ok(HuntNext::Stay);
            }
            Some(MoveTo::Arrived) => return self.walk_arrived(c, unit, f, r),
            None => Err(GenieError::unsupported(
                "a Hunt in state 4 without its MoveTo",
            )),
        }
        .map(|()| HuntNext::Stay)
    }

    /// The `MoveTo`'s start (case 3; after a wait `+0x6c` goes from 1 to 2 as it asks again):
    /// the walk to the target within the walk range, what stands in the way (posted at once),
    /// and the hunt's walking sprite (the `MoveTo`'s `set_state(4)`); without a way it gives up.
    fn walk_start(
        &mut self,
        c: &mut Step<'_>,
        unit: &mut Unit,
        me: Me,
        f: &Fight,
        r: &HuntReads,
    ) -> Result<(), GenieError> {
        if self.move_wait.state == 1 {
            self.move_wait.state = 2;
        }
        self.move_to = Some(MoveTo::Moving);
        let range = self.walk_range;
        let goal = Goal::Target(self.target);
        let graphic = walk_sprite(
            self.task.resource,
            unit.life.attribute_type,
            unit.life.amount,
            &r.sprites,
        );
        let me = Me {
            action_state: self.state,
            ..me
        };
        let (walk, sprite_rng, _) = c.walk_sprites_action(&mut unit.body, me);
        let shown = Shown {
            sprite: &mut unit.sprite,
            rng: sprite_rng,
            graphic,
        };
        let ask = PathAsk::default();
        let walked = start_walk_shown(walk, (goal, range, true), ask, Some(shown))?;
        unit.post_in_the_way()?;
        match walked {
            Some(state) => self.move_to = Some(state),
            None => self.give_up(&r.attack, f.own.class),
        }
        Ok(())
    }

    /// The `MoveTo` on its way: a target that moved makes it start again; else the walk's step,
    /// what stands in the way (posted at once), and without a way the give-up.
    fn walk_moving(
        &mut self,
        c: &mut Step<'_>,
        unit: &mut Unit,
        me: Me,
        f: &Fight,
        r: &AttackReads,
    ) -> Result<(), GenieError> {
        if target_moved(&c.walk(&mut unit.body, me), self.target, true)?.again() {
            self.move_to = Some(MoveTo::Start);
            return Ok(());
        }
        let me = Me {
            action_state: self.state,
            ..me
        };
        let walked = walk_or_give_up(c.walk(&mut unit.body, me))?;
        unit.post_in_the_way()?;
        match walked {
            Some(m) => self.move_to = Some(m),
            None => self.give_up(r, f.own.class),
        }
        Ok(())
    }

    /// The failed `MoveTo`'s update returns 3 (quiet: no notify): `FUN_140ac4670` with 8 and -1
    /// looks for another target only for a class the unit AI's slot 0x1b0 gives 0
    /// (`FUN_140c1c4f0`: 15, trees; the class its unit AI targets, else the hunt's: the caller's
    /// search, `Unit::hunt_tree_search`, which found none); then the hunt waits 3 seconds and the
    /// unit's animation stops (`FUN_141161850`).
    fn walk_failed(&mut self, unit: &mut Unit, r: &HuntReads) -> Result<(), GenieError> {
        self.no_tree_search(r)?;
        self.timer = -3.0;
        set_walker_anim(&mut unit.body, 0.0);
        Ok(())
    }

    /// The `MoveTo` arrived: its update in state 1 returns 1 (the list deletes it, and its
    /// destructor `0x140aa1f30` stops the unit, slot 0x5d0); the decay check (`FUN_140ac44b0`)
    /// and `set_state(6)`.
    fn walk_arrived(
        &mut self,
        c: &mut Step<'_>,
        unit: &mut Unit,
        f: &mut Fight,
        r: &HuntReads,
    ) -> Result<HuntNext, GenieError> {
        self.move_to = None;
        unit.body
            .mover_mut()
            .ok_or(crate::world::HUNTER_UNMOVING)?
            .stop()?;
        self.decay_check(c, r.stops_decay, f)?;
        self.set_face(c, unit, f, r)
    }

    /// Case 5, the face (`FUN_141162db0`; a unit without a turn time turns at once, by the
    /// facets between; one that turns over time is not modelled): a target holding something
    /// is gathered (`set_state(7)`); an empty one goes (slot 0x68 with none, the caller's) and
    /// the hunt searches (`set_state(3)`).
    fn update_face(
        &mut self,
        c: &mut Step<'_>,
        unit: &mut Unit,
        f: &Fight,
        r: &HuntReads,
    ) -> Result<HuntNext, GenieError> {
        self.face_animal(unit, f.victim.position, r.attack.world_time)?;
        if 0.0 < f.victim.amount {
            self.set_gather(unit, c.w.streams.get(Stream::Sprite), r);
            return Ok(HuntNext::Stay);
        }
        self.state = SEARCH;
        Ok(HuntNext::Search)
    }

    /// State 4's answer to its `MoveTo` losing the target (codes 4 and 5): `FUN_140ac4670` with 8
    /// and -1 looks for another target of a class the unit AI's slot 0x1b0 gives 0 (trees: the
    /// caller's search, `Unit::hunt_tree_search`, which found none); then the target goes (slot
    /// 0x68 with none) and the hunt walks to its point (`set_state` 11).
    pub(super) fn lost_on_the_way(
        &mut self,
        mover: &mut Mover,
        r: &HuntReads,
    ) -> Result<(), GenieError> {
        self.no_tree_search(r)?;
        self.set_point_walk(WALK_SEARCH, mover)
    }

    /// `FUN_140ac4670` with 8 and −1 for a class the unit AI's slot 0x1b0 gives 0 (trees: the
    /// class its unit AI targets, else the hunt's): the caller's search
    /// (`Unit::hunt_tree_search`) found none; another one here is not modelled.
    fn no_tree_search(&self, r: &HuntReads) -> Result<(), GenieError> {
        if let Some(class) = r.ai_class {
            let class = if class == -1 {
                i32::from(self.target_class)
            } else {
                class
            };
            if class == TREE_CLASS && !r.tree_searched {
                return Err(GenieError::unsupported(
                    "a hunt's search for another tree (FUN_140ac4670)",
                ));
            }
        }
        Ok(())
    }

    /// `set_state(12)` on live livestock: the Attack of the master's ranges starts (with its
    /// target in reach its point and the facing, [`Attack::start`]), showing the idle sprite when
    /// its `MoveTo` starts arrived.
    pub(super) fn start_attack(
        &mut self,
        c: &mut Step<'_>,
        unit: &mut Unit,
        herd: &Herd,
    ) -> Result<(), GenieError> {
        let mover = unit.body.mover_mut().ok_or(crate::world::HUNTER_UNMOVING)?;
        self.move_to = None;
        self.state = ATTACK;
        let (a, arrived) = Attack::start(self.target, &herd.engagement, herd.arms, mover)?;
        if arrived {
            a.arrived_sprite(unit, c.w.streams.get(Stream::Sprite))?;
        }
        self.attack = Some(a);
        Ok(())
    }
}

/// The hunt's target back to the world where its own part changed it (its gathering, its
/// decay's stop), and a kill it left ([`crate::combat::victims::write_back`]).
///
/// # Errors
///
/// The write-back's and the kill's.
pub(super) fn write_target_back(c: &mut Step<'_>, f: &mut Fight) -> Result<(), GenieError> {
    if let Some(k) = crate::combat::victims::write_back(c, &mut f.victim)? {
        crate::combat::victims::file_kill(c, k)?;
    }
    Ok(())
}
