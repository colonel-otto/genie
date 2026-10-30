//! The unit's own Attack in its update: a lost target's end, and the strike with what
//! follows.

use crate::GenieError;
use crate::combat::attack::{AttackReads, Fight};
use crate::combat::damage::Victim;
use crate::combat::victims::{KillLeft, file_kill, victim, victim_of};
use crate::combat::volley::armed;
use crate::math::rng::Stream;
use crate::movement::Me;
use crate::objects::lifecycle::seconds;
use crate::objects::teardown::HeadCode;
use crate::objects::unit::Unit;
use crate::objects::unit::object_master;
use crate::world::step::Step;

/// The unit's own Attack's update without its Attack: the caller's test.
const NO_ATTACK: GenieError = GenieError::unreachable("an Attack's update without its Attack");

impl Unit {
    /// The unit's own Attack in its update: after its target went
    /// ([`crate::combat::attack::Attack::lost`]), or its update (slot 6, `0x140a82ae0`) with the
    /// strike and what follows; codes 1 and 3 delete it (`RGE_Action_Object::update`). `own` is
    /// the unit as a combat object. The strike reads its target and the blast's objects from
    /// the world and writes them back there right after the hit
    /// ([`crate::combat::victims::write_strike_back`]); the unit's own kill by a retaliation is
    /// the caller's after the update.
    pub(crate) fn attack_part(
        &mut self,
        c: &mut Step<'_>,
        me: Me,
        own: Option<Victim>,
    ) -> Result<HeadCode, GenieError> {
        match self.actions.attack().map(|a| a.target) {
            None => Ok(HeadCode::Keeps),
            Some(-1) => self.attack_lost_part(c),
            Some(_) => self.attack_strike_part(c, me, own),
        }
    }

    /// The unit's own Attack after its target went
    /// ([`crate::combat::attack::Attack::lost`]): its update without a target, the notify its
    /// disarm posts to the unit AI; codes 1 and 3 delete it.
    fn attack_lost_part(&mut self, c: &mut Step<'_>) -> Result<HeadCode, GenieError> {
        let master = self.master(&c.w.masters, c.player)?;
        let armed = armed(master, self.firepower);
        let attack = self.actions.attack_mut().ok_or(NO_ATTACK)?;
        let (code, disarmed) = attack.update_targetless(seconds(c.dt), armed, &mut self.body)?;
        if disarmed {
            let posted = attack.disarmed(
                &mut self.sprite,
                &mut self.body,
                c.w.streams.get(Stream::Sprite),
            )?;
            if let Some((kind, p)) = posted {
                self.post_own(kind, p)?;
            }
        }
        Ok(HeadCode::of(code))
    }

    /// The unit's own Attack's update (slot 6, `0x140a82ae0`) on the step: what it reads of the
    /// unit ([`AttackReads::of`]), its target and the unit as combat objects ([`Fight`]); the
    /// Attack, copied out of the list, goes back after it, then the retaliation's changes to the
    /// unit ([`Unit::hit_back`]). Codes 1 and 3 delete it (`RGE_Action_Object::update`).
    fn attack_strike_part(
        &mut self,
        c: &mut Step<'_>,
        me: Me,
        own: Option<Victim>,
    ) -> Result<HeadCode, GenieError> {
        let mut attack = *self.actions.attack().ok_or(NO_ATTACK)?;
        let master = object_master(
            self.own_master.as_deref(),
            (self.id, self.life.master_id),
            c.w.masters.get(c.player),
        )?;
        let computer = c.w.players.get(c.player).is_some_and(|q| q.computer);
        let r = AttackReads::of(
            self,
            master,
            (c.w.masters.get(c.player), &c.w.graphics),
            self.ai.as_ref(),
            (computer, c.w.world_time),
        );
        let victim = victim(c, attack.target)?.ok_or(GenieError::unsupported(
            "an attack's target the slice does not hold live",
        ))?;
        let own = own.ok_or(crate::world::ATTACKER_MASTER)?;
        let mut f = Fight { victim, own };
        // written back before an error too: the stop's context reads the list
        let code = attack.update(c, self, me, &mut f, &r);
        if let Some(a) = self.actions.attack_mut() {
            *a = attack;
        }
        let code = code?;
        // the retaliation's kill of the unit itself, which follows its step
        if let Some(k) = self.hit_back(f.own)? {
            file_kill(c, self, KillLeft::own(self.id, k))?;
        }
        Ok(HeadCode::of(code))
    }
}

impl Unit {
    /// `RGE_Combat_Object::update`'s timers (`0x14107dfb0`, before the actions): the attack timer
    /// runs down, and a reload world time that passed ends the volley under way.
    pub(crate) fn run_down_attack_timers(&mut self, c: &Step<'_>) {
        // RGE_Combat_Object::update (0x14107dfb0): the attack timer runs down by dt times
        // `+0x87c` (a terrain look's rate, [`crate::combat::abilities`])
        if 0.0 < self.attack_timer {
            self.attack_timer -= seconds(c.dt) * self.abilities.attack_rate;
            if self.attack_timer < 0.0 {
                self.attack_timer = 0.0;
            }
        }
        // then a reload world time past ends the volley under way (`+0xa04`) and is none
        if self.reload_worldtime != 0 && self.reload_worldtime < c.w.world_time {
            self.next_volley = 0;
            self.reload_worldtime = 0;
        }
    }

    /// The hunter or the attacker as a combat object, which its hits read and change: none for a
    /// unit without a Hunt or an Attack, or without its master.
    pub(crate) fn own_victim(&self, c: &Step<'_>) -> Option<Victim> {
        (self.actions.hunt().is_some() || self.actions.attack().is_some())
            .then_some(())
            .and_then(|()| {
                let master = self.master(&c.w.masters, c.player).ok()?;
                let terrain = c.w.spatial.terrain.as_ref();
                Some(victim_of(
                    self,
                    master,
                    &c.w.players[c.player].attributes,
                    c.player,
                    terrain,
                    c.w.mode_680,
                ))
            })
    }
}
