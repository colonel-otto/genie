//! The unit's own Attack in its update: a lost target's end, and the strike with what
//! follows.

use crate::GenieError;
use crate::combat::attack::Strike;
use crate::combat::damage::Victim;
use crate::combat::victims::{KillLeft, file_kill, victim, victim_of, write_struck};
use crate::combat::volley::{Aimed, armed};
use crate::math::rng::Stream;
use crate::movement::Me;
use crate::objects::lifecycle::seconds;
use crate::objects::teardown::HeadCode;
use crate::objects::unit::Unit;
use crate::objects::unit::object_master;
use crate::unit_ai::ACTION_EXECUTED;
use crate::world::step::Step;

impl Unit {
    /// The unit's own Attack in its update: after its target went
    /// ([`crate::combat::attack::Attack::lost`]), or its update (slot 6, `0x140a82ae0`) with the
    /// strike and what follows; codes 1 and 3 delete it (`RGE_Action_Object::update`). `own` is
    /// the unit as a combat object. The strike reads its target and the blast's objects from
    /// the world and writes them back there right after it
    /// ([`write_back`]); their kills, the unit's own by a retaliation between them, are the
    /// caller's after the update.
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
        let attack = self.actions.attack_mut().expect("the caller's test");
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

    /// The unit's own Attack's update (slot 6, `0x140a82ae0`) with the strike and what follows:
    /// its notifies posted to its unit AI (slot 0x328); codes 1 and 3 delete it
    /// (`RGE_Action_Object::update`).
    fn attack_strike_part(
        &mut self,
        c: &mut Step<'_>,
        me: Me,
        own: Option<Victim>,
    ) -> Result<HeadCode, GenieError> {
        let attack = self.actions.attack_mut().expect("the caller's test");
        let master = object_master(
            self.own_master.as_deref(),
            (self.id, self.life.master_id),
            c.w.masters.get(c.player),
        )?;
        let armed = armed(master, self.firepower);
        let computer = c.w.players.get(c.player).is_some_and(|q| q.computer);
        let mut victim = victim(c, attack.target).ok_or(GenieError::unsupported(
            "an attack's target the slice does not hold live",
        ))?;
        let own = own.ok_or(crate::world::ATTACKER_MASTER)?;
        let mut blasted = std::mem::take(&mut c.blasted);
        let can_move = self
            .ai
            .as_ref()
            .is_some_and(|ai| ai.current_order.is_some() || ai.stance <= 1);
        let stance = self.ai.as_ref().map_or(0, |ai| ai.stance);
        let world_time = c.w.world_time;
        let standing = c.w.graphics.get(&master.standing_sprite).copied();
        let volleys =
            c.w.masters
                .get(c.player)
                .is_some_and(|m| crate::combat::attack::volleys(master, m));
        let swing_sprites = crate::combat::attack::swing_sprites(
            self.abilities.look.as_ref(),
            master,
            &c.w.graphics,
        );
        let (reload, hit_chance) = (master.reload, master.hit_chance);
        let (walk, sprite_rng) = c.walk_and_sprites(&mut self.body, me);
        let mut strike = Strike {
            sprite: &mut self.sprite,
            standing,
            volleys,
            attack_timer: &mut self.attack_timer,
            reload_worldtime: &mut self.reload_worldtime,
            reload: self.reload_stinger + (reload - self.reload_powerup),
            hit_chance,
            victim: &mut victim,
            blasted: &mut blasted,
            sprite_rng,
            own,
            executed: None,
            fire: None,
            can_move,
            stance,
            state_position: self.ai.as_ref().map_or([0.0; 2], |ai| ai.state_position),
            posted: Vec::new(),
            queue_empty: self.ai.as_ref().is_some_and(|ai| ai.orders.is_empty()),
            world_time,
            ai_order: self.ai.as_ref().map(|ai| ai.current_order),
            ai_type: self.ai.as_ref().map(|ai| ai.ai_type),
            computer,
            armed,
            volley_under_way: self.next_volley != 0,
            let_go: None,
            forgets: false,
            sends_home: false,
            swing_sprites,
        };
        let code = attack.update(walk, &mut strike)?;
        let fired = strike.fire.map(|target| Aimed::at(target, strike.victim));
        let (executed, own) = (strike.executed, std::mem::take(&mut strike.own));
        let posted = std::mem::take(&mut strike.posted);
        let (let_go, forgets) = (strike.let_go, strike.forgets);
        let sends_home = strike.sends_home;
        write_struck(c, blasted, &mut victim)?;
        if let Some(t) = let_go {
            let by = self.releaser(c);
            crate::world::lets::release(c, t, by)?;
        }
        if forgets && let Some(ai) = self.ai.as_mut() {
            ai.remove_target();
        }
        if sends_home && let Some(home) = self.ai.as_mut().and_then(|ai| ai.home.as_mut()) {
            home.go_home = true;
        }
        for (kind, p) in posted {
            self.post_own(kind, p)?;
        }
        if let Some(aimed) = fired {
            self.volley(c, aimed)?;
        }
        if let Some(target) = executed {
            self.post_own(ACTION_EXECUTED, [600, target, 0])?;
        }
        // the retaliation's kill of the unit itself, which follows its step
        if let Some(k) = self.hit_back(own)? {
            file_kill(c, KillLeft::own(self.id, k))?;
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
