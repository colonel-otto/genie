//! Leading own livestock to its drop site (state 10), facing an animal and the decay check.

use crate::GenieError;
use crate::combat::attack::{Strike, set_sprite};
use crate::combat::damage::Victim;
use crate::gathering::hunt::{
    DECAY_TASK, Gatherer, Herd, HuntAction, LEAD, LEAD_REACH, LEAD_WAIT, SEARCH, VILLAGER_CLASS,
};
use crate::movement::footprint_gap_squared;
use crate::movement::move_to::Walk;

impl HuntAction {
    /// `set_state(10)` (`0x140ac60e0` case 10): `sheep_still_time` clears, the animal's place
    /// becomes the start (`+0x78`, `+0x7c`), the hunter faces it (`FUN_141162db0`, at once; one
    /// that turns over time is not modelled), a hunter of type 70 with a special graphic shows
    /// it (slot 0x68 with master `+0x8e8`) and the timer starts at −1 s.
    pub(super) fn set_lead(
        &mut self,
        mut walk: Walk<'_>,
        strike: &mut Strike<'_>,
        herd: &Herd,
    ) -> Result<(), GenieError> {
        self.tear_down(walk.body.mover_mut())?;
        self.state = LEAD;
        self.lead.still_time = 0;
        let at = strike.victim.position;
        self.lead.start = [at[0], at[1]];
        self.face_animal(&mut walk, strike)?;
        if let Some(g) = herd.special_graphic {
            set_sprite(strike.sprite, g, strike.sprite_rng);
        }
        self.timer = LEAD_WAIT;
        Ok(())
    }

    /// `FUN_141162db0` on the target at once (a unit without a turn time turns by the facets
    /// between).
    pub(super) fn face_animal(
        &self,
        walk: &mut Walk<'_>,
        strike: &Strike<'_>,
    ) -> Result<(), GenieError> {
        let at = walk.position();
        let mover = walk.body.mover_mut().ok_or(crate::world::HUNTER_UNMOVING)?;
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
        Ok(())
    }

    /// State 10's update (`0x140ac7a00` case 9): without its target the hunt searches
    /// (`set_state(3)`, the caller's). An animal that stands where it was sent (`Herd::idle`) is
    /// slaughtered (`set_state(12)`); else the timer runs by dt and at its end an animal on the
    /// start is slaughtered too; one that moved but stays within 1.5 tiles (`LEAD_REACH`
    /// squared, `FUN_14118f8c0`) becomes the start again, the hunter faces it and waits another
    /// second; one farther is walked to (`set_state(4)`).
    pub(super) fn update_lead(
        &mut self,
        mut walk: Walk<'_>,
        strike: &mut Strike<'_>,
        gather: &mut Gatherer<'_>,
        herd: Option<Herd>,
    ) -> Result<(), GenieError> {
        if self.target == -1 {
            self.state = SEARCH;
            gather.searches = true;
            return Ok(());
        }
        let herd = herd.ok_or(GenieError::unsupported(
            "a hunt leading livestock unprepared",
        ))?;
        if herd.idle {
            return self.start_attack(walk, strike, &herd);
        }
        self.timer += walk.dt;
        if self.timer < 0.0 {
            return Ok(());
        }
        let v = strike.victim.position;
        if v[0] == self.lead.start[0] && v[1] == self.lead.start[1] {
            return self.start_attack(walk, strike, &herd);
        }
        let at = walk.position();
        let ground = walk
            .body
            .mover()
            .map(|m| [m.stats.radius[0], m.stats.radius[1]])
            .ok_or(crate::world::HUNTER_UNMOVING)?;
        if footprint_gap_squared(at, ground, v, strike.victim.radius) <= LEAD_REACH {
            self.lead.start = [v[0], v[1]];
            self.face_animal(&mut walk, strike)?;
            self.timer = LEAD_WAIT;
            return Ok(());
        }
        let player = strike.own.player;
        self.set_walk(&mut walk, strike.victim, player)
    }

    /// `FUN_140ac44b0`: a task whose `attribute_type4` is 0x124, with its player's attribute 292
    /// at 1, stops its target's decay: a target whose master decays (`+0x620` above 0) gets a
    /// master of its own unless it has one (`FUN_141188bc0`, `FUN_141186d10`) and that master's
    /// decay takes the other sign (its bits XOR `0x80000000`); the caller writes it back to the
    /// target ([`Victim::decay`]). Otherwise it does nothing.
    pub(super) fn decay_check(&self, stops_decay: bool, victim: &mut Victim) {
        if self.task.kind4 == DECAY_TASK && stops_decay && 0.0 < victim.decay {
            victim.decay = f32::from_bits(victim.decay.to_bits() ^ 0x8000_0000);
        }
    }

    /// The `MoveTo`'s answer to the motion step giving up ([`MoveWait::give_up`]; a stance below 2
    /// without a unit AI; quiet).
    ///
    /// [`MoveWait::give_up`]: crate::movement::move_to::MoveWait::give_up
    pub(super) const fn give_up(&mut self, strike: &Strike<'_>) {
        let waits = !strike.computer && strike.own.class == VILLAGER_CLASS;
        self.move_to = Some(self.move_wait.give_up(waits, strike.stance));
    }
}
