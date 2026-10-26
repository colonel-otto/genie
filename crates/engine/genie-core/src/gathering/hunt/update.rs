//! One update (slot 6, `0x140ac7a00`), a target lost on the way and the attack on live
//! livestock (state 12).

use crate::GenieError;
use crate::combat::attack::{Attack, Strike};
use crate::gathering::hunt::{
    ATTACK, FACE, GATHER, Gatherer, Herd, HuntAction, LEAD, PREDATOR, SEARCH, Shown, TREE_CLASS,
    WALK, WALK_SEARCH, walk_sprite,
};
use crate::movement::move_to::{
    Goal, MoveTo, Walk, start_walk_or_give_up, target_moved, walk_or_give_up,
};

impl HuntAction {
    /// One update (slot 6, `0x140ac7a00`): state 12 updates the Attack (`FUN_140a992b0`); its end
    /// on the dead target (code 5) runs `FUN_140ac44b0` and `set_state(4)`. State 4 updates the
    /// `MoveTo`; its end (code 1) runs `FUN_140ac44b0` and `set_state(6)`. State 6 faces the target
    /// (`FUN_141162db0`) and, while it holds something, `set_state(7)`; state 7 gathers. A computer
    /// player's hunt and the other codes are not modelled.
    pub fn update(
        &mut self,
        mut walk: Walk<'_>,
        computer: bool,
        strike: &mut Strike<'_>,
        gather: &mut Gatherer<'_>,
        herd: Option<Herd>,
    ) -> Result<bool, GenieError> {
        if computer {
            return Err(GenieError::unsupported(
                "a computer player's hunt (FUN_140fde4f0)",
            ));
        }
        let mut full = false;
        // a target gone or past state 6 went at the update's start (slot 0x68 with none, the
        // caller's): in state 4 its `MoveTo`, quiet, forgets it too and returns 5
        if self.state == WALK && self.target == -1 && 0.0 <= self.timer && self.move_to.is_some() {
            self.lost_on_the_way(&mut walk, gather)?;
            return Ok(false);
        }
        let r = match self.state {
            ATTACK => self.update_attack(walk, strike, gather),
            // case 3: a hunt that gave up on its MoveTo waits (its timer below 0), then set_state 11
            WALK if self.timer < 0.0 => {
                let mover = walk
                    .mover
                    .as_deref_mut()
                    .ok_or(crate::world::HUNTER_UNMOVING)?;
                self.wait_then_walk(walk.dt, mover)
            }
            WALK => self.update_walk(walk, strike, gather, herd),
            LEAD => self.update_lead(walk, strike, gather, herd),
            FACE => self.update_face(walk, strike, gather),
            // case 6 without a target (or a task): set_state(8) to carry the load off, set_state(3)
            // with none
            GATHER if self.target == -1 => {
                full = self.ends_without_target(*gather.held);
                if !full {
                    gather.searches = true;
                }
                Ok(())
            }
            GATHER => {
                full = self.gather(walk.dt, strike.victim, gather)?;
                Ok(())
            }
            _ => Err(GenieError::unsupported(
                "a Hunt action in a state the slice does not model",
            )),
        };
        r.map(|()| full)
    }

    /// State 12, the Attack's update (`FUN_140a992b0`) and its end: ended (codes 1, 2 and 5)
    /// with no target left the hunt searches (`set_state(3)`); a predator (class 10) of a unit AI
    /// with orders queued (`FUN_140c564a0` above 0) would end the hunt (`set_state(1)`) under
    /// the game's setting `+0x98`, not modelled; else the decay check (`FUN_140ac44b0`) and the
    /// walk to the target (`set_state(4)`). Failed (3, its `MoveTo` did) the caller's search for
    /// the next tree follows, else `set_state(3)`; the target gone inside another (code 4,
    /// `LAB_140ac7ecd`) is let go and the hunt walks to its point to search there
    /// (`set_state(11)`).
    fn update_attack(
        &mut self,
        mut walk: Walk<'_>,
        strike: &mut Strike<'_>,
        gather: &mut Gatherer<'_>,
    ) -> Result<(), GenieError> {
        let attack = self.attack.as_mut().ok_or(GenieError::unsupported(
            "a Hunt in state 12 without its Attack",
        ))?;
        match attack.update(walk.reborrow(), strike)? {
            0 | 6.. => Ok(()),
            1 | 2 | 5 if self.target == -1 => {
                self.tear_down(walk.mover.as_deref_mut())?;
                self.state = SEARCH;
                gather.searches = true;
                Ok(())
            }
            1 | 2 | 5
                if gather.predator_hunts_end
                    && strike.victim.class == PREDATOR
                    && !strike.queue_empty =>
            {
                Err(GenieError::unsupported(
                    "a hunt that ends with its Attack on a predator (set_state 1)",
                ))
            }
            1 | 2 | 5 => {
                self.decay_check(gather.stops_decay, strike.victim);
                let player = strike.own.player;
                self.set_walk(&mut walk, strike.victim, player)
            }
            3 => {
                gather.attack_failed = true;
                Ok(())
            }
            _ => {
                gather.let_go = Some(self.let_go());
                let mover = walk
                    .mover
                    .as_deref_mut()
                    .ok_or(crate::world::HUNTER_UNMOVING)?;
                self.set_point_walk(WALK_SEARCH, mover)
            }
        }
    }

    /// State 4, the walk's `MoveTo` (`FUN_140a992b0`): it starts ([`HuntAction::walk_start`]),
    /// walks ([`HuntAction::walk_moving`]), fails ([`HuntAction::walk_failed`]), waits (case 2:
    /// the timer runs by dt; at its end it asks again, `set_state(3)`) or arrives
    /// ([`HuntAction::walk_arrived`]).
    fn update_walk(
        &mut self,
        walk: Walk<'_>,
        strike: &mut Strike<'_>,
        gather: &mut Gatherer<'_>,
        herd: Option<Herd>,
    ) -> Result<(), GenieError> {
        match self.move_to {
            Some(MoveTo::Start) => self.walk_start(walk, strike, gather),
            Some(MoveTo::Moving) => self.walk_moving(walk, strike),
            Some(MoveTo::Failed) => self.walk_failed(strike, gather),
            Some(MoveTo::Waiting) => {
                self.move_wait.timer += walk.dt;
                if self.move_wait.delay <= self.move_wait.timer {
                    self.move_wait.timer = 0.0;
                    self.move_to = Some(MoveTo::Start);
                }
                Ok(())
            }
            Some(MoveTo::Arrived) => self.walk_arrived(walk, strike, gather, herd),
            None => Err(GenieError::unsupported(
                "a Hunt in state 4 without its MoveTo",
            )),
        }
    }

    /// The `MoveTo`'s start (case 3; after a wait `+0x6c` goes from 1 to 2 as it asks again):
    /// the walk to the target within the walk range, what stands in the way, and the hunt's
    /// walking sprite (the `MoveTo`'s `set_state(4)`); without a way it gives up.
    fn walk_start(
        &mut self,
        mut walk: Walk<'_>,
        strike: &mut Strike<'_>,
        gather: &Gatherer<'_>,
    ) -> Result<(), GenieError> {
        if self.move_wait.state == 1 {
            self.move_wait.state = 2;
        }
        self.move_to = Some(MoveTo::Moving);
        let range = self.walk_range;
        walk.me.action_state = self.state;
        let goal = Goal::Target(self.target);
        let walked = start_walk_or_give_up(walk.reborrow(), goal, range, true)?;
        crate::combat::attack::in_the_way(&mut walk, strike);
        let shows = walk_sprite(
            self.task.resource,
            *gather.held_kind,
            *gather.held,
            &gather.sprites,
        );
        Shown {
            sprite: strike.sprite,
            rng: strike.sprite_rng,
            graphic: shows,
        }
        .walking(walked.unwrap_or(MoveTo::Moving));
        match walked {
            Some(state) => self.move_to = Some(state),
            None => self.give_up(strike),
        }
        Ok(())
    }

    /// The `MoveTo` on its way: a target that moved makes it start again; else the walk's step,
    /// what stands in the way, and without a way the give-up.
    fn walk_moving(
        &mut self,
        mut walk: Walk<'_>,
        strike: &mut Strike<'_>,
    ) -> Result<(), GenieError> {
        if target_moved(&walk, self.target, true)?.again() {
            self.move_to = Some(MoveTo::Start);
            return Ok(());
        }
        walk.me.action_state = self.state;
        let walked = walk_or_give_up(walk.reborrow())?;
        crate::combat::attack::in_the_way(&mut walk, strike);
        match walked {
            Some(m) => self.move_to = Some(m),
            None => self.give_up(strike),
        }
        Ok(())
    }

    /// The failed `MoveTo`'s update returns 3 (quiet: no notify): `FUN_140ac4670` with 8 and -1
    /// looks for another target only for a class the unit AI's slot 0x1b0 gives 0
    /// (`FUN_140c1c4f0`: 15, trees; the class its unit AI targets, else the hunt's: the caller's
    /// search, `Unit::hunt_tree_search`, which found none); then the hunt waits 3 seconds and the
    /// unit's animation stops (`FUN_141161850`).
    fn walk_failed(
        &mut self,
        strike: &mut Strike<'_>,
        gather: &Gatherer<'_>,
    ) -> Result<(), GenieError> {
        if let Some(class) = gather.ai_class {
            let class = if class == -1 {
                i32::from(self.target_class)
            } else {
                class
            };
            if class == TREE_CLASS && !gather.tree_searched {
                return Err(GenieError::unsupported(
                    "a hunt's search for another tree (FUN_140ac4670)",
                ));
            }
        }
        self.timer = -3.0;
        *strike.anim_speed = Some(0.0);
        Ok(())
    }

    /// The `MoveTo` arrived: its update in state 1 returns 1 (the list deletes it, and its
    /// destructor `0x140aa1f30` stops the unit, slot 0x5d0); the decay check (`FUN_140ac44b0`)
    /// and `set_state(6)`.
    fn walk_arrived(
        &mut self,
        mut walk: Walk<'_>,
        strike: &mut Strike<'_>,
        gather: &mut Gatherer<'_>,
        herd: Option<Herd>,
    ) -> Result<(), GenieError> {
        self.move_to = None;
        walk.mover
            .as_deref_mut()
            .ok_or(crate::world::HUNTER_UNMOVING)?
            .stop()?;
        self.decay_check(gather.stops_decay, strike.victim);
        self.set_face(walk, strike, gather, herd)
    }

    /// Case 5, the face (`FUN_141162db0`; a unit without a turn time turns at once, by the
    /// facets between; one that turns over time is not modelled): a target holding something
    /// is gathered (`set_state(7)`); an empty one goes (slot 0x68 with none, the caller's) and
    /// the hunt searches (`set_state(3)`).
    fn update_face(
        &mut self,
        mut walk: Walk<'_>,
        strike: &mut Strike<'_>,
        gather: &mut Gatherer<'_>,
    ) -> Result<(), GenieError> {
        let at = walk
            .world
            .get(walk.me.id)
            .map_or(walk.me.position, |p| p.position);
        let mover = walk
            .mover
            .as_deref_mut()
            .ok_or(crate::world::HUNTER_UNMOVING)?;
        if !crate::combat::attack::face_target(
            mover,
            at,
            strike.victim.position,
            strike.world_time,
        )? {
            return Err(GenieError::unsupported(
                "a hunter that turns over time to face its target",
            ));
        }
        if 0.0 < strike.victim.amount {
            self.set_gather(strike, gather);
        } else {
            self.state = SEARCH;
            gather.searches = true;
        }
        Ok(())
    }

    /// State 4's answer to its `MoveTo` losing the target (codes 4 and 5): `FUN_140ac4670` with 8
    /// and -1 looks for another target of a class the unit AI's slot 0x1b0 gives 0 (trees: the
    /// caller's search, `Unit::hunt_tree_search`, which found none); then the target goes (slot
    /// 0x68 with none) and the hunt walks to its point (`set_state` 11).
    pub(super) fn lost_on_the_way(
        &mut self,
        walk: &mut Walk<'_>,
        gather: &Gatherer<'_>,
    ) -> Result<(), GenieError> {
        if let Some(class) = gather.ai_class {
            let class = if class == -1 {
                i32::from(self.target_class)
            } else {
                class
            };
            if class == TREE_CLASS && !gather.tree_searched {
                return Err(GenieError::unsupported(
                    "a hunt's search for another tree (FUN_140ac4670)",
                ));
            }
        }
        let mover = walk
            .mover
            .as_deref_mut()
            .ok_or(crate::world::HUNTER_UNMOVING)?;
        self.set_point_walk(WALK_SEARCH, mover)
    }

    /// `set_state(12)` on live livestock: the Attack of the master's ranges starts (with its
    /// target in reach its point and the facing, [`Attack::start`]), showing the idle sprite when
    /// its `MoveTo` starts arrived.
    pub(super) fn start_attack(
        &mut self,
        walk: Walk<'_>,
        strike: &mut Strike<'_>,
        herd: &Herd,
    ) -> Result<(), GenieError> {
        let mover = walk.mover.ok_or(crate::world::HUNTER_UNMOVING)?;
        self.move_to = None;
        self.state = ATTACK;
        let (a, arrived) = Attack::start(self.target, &herd.engagement, herd.arms, mover)?;
        if arrived {
            a.arrived_sprite(strike)?;
        }
        self.attack = Some(a);
        Ok(())
    }
}
