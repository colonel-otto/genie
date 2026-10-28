//! The unit's own Attack in its update: a lost target's end, and the strike with what
//! follows.

use crate::GenieError;
use crate::combat::attack::Strike;
use crate::combat::damage::Victim;
use crate::combat::victims::victim_of;
use crate::combat::volley::{Aimed, armed};
use crate::math::rng::Stream;
use crate::movement::Me;
use crate::objects::unit::Unit;
use crate::unit_ai::ACTION_EXECUTED;
use crate::world::context::Context;

impl Unit {
    /// The unit's own Attack in its update: after its target went
    /// ([`crate::combat::attack::Attack::lost`]), or its update (slot 6, `0x140a82ae0`) with the
    /// strike and what follows; codes 1 and 3 delete it (`RGE_Action_Object::update`). `own` is
    /// the unit as a combat object.
    pub(crate) fn attack_part(
        &mut self,
        c: &mut Context<'_>,
        me: Me,
        own: Option<Victim>,
    ) -> Result<(), GenieError> {
        if let Some(attack) = self.actions.attack_mut().filter(|a| a.target == -1) {
            // the unit's own Attack after its target went ([`Attack::lost`])
            let master = c
                .masters
                .and_then(|m| m.get(self.life.master_id))
                .ok_or(crate::world::ATTACKER_MASTER)?;
            let armed = armed(master, self.firepower);
            let (code, disarmed) = attack.update_targetless(c.dt_seconds, armed, &mut self.body)?;
            if disarmed {
                let posted = attack.disarmed(
                    &mut self.sprite,
                    &mut self.body,
                    c.streams.get(Stream::Sprite),
                )?;
                if let Some((kind, p)) = posted {
                    self.post_own(kind, p)?;
                }
            }
            if matches!(code, 1 | 3) {
                self.delete_attack(c)?;
            }
        } else if let Some(attack) = self.actions.attack_mut() {
            // the unit's own Attack (slot 6, 0x140a82ae0), its notifies posted to its unit AI
            // (slot 0x328); codes 1 and 3 delete it (RGE_Action_Object::update)
            let master = c
                .masters
                .and_then(|m| m.get(self.life.master_id))
                .ok_or(crate::world::ATTACKER_MASTER)?;
            let armed = armed(master, self.firepower);
            let computer = c.computers.get(c.player).copied().unwrap_or(false);
            let victim = c.victim.take().ok_or(GenieError::unsupported(
                "an attack's target the slice does not hold live",
            ))?;
            let own = own.ok_or(crate::world::ATTACKER_MASTER)?;
            let mut blasted = std::mem::take(&mut c.blasted);
            let can_move = self
                .ai
                .as_ref()
                .is_some_and(|ai| ai.current_order != -1 || ai.stance <= 1);
            let stance = self.ai.as_ref().map_or(0, |ai| ai.stance);
            let world_time = c.world_time;
            let standing = c.graphics.get(&master.standing_sprite).copied();
            let volleys = c
                .masters
                .is_some_and(|m| crate::combat::attack::volleys(master, m));
            let swing_sprites = crate::combat::attack::swing_sprites(
                self.abilities.look.as_ref(),
                master,
                c.graphics,
            );
            let (walk, sprite_rng) = c.walk_and_sprites(self.body.mover_mut(), me);
            let mut strike = Strike {
                sprite: &mut self.sprite,
                standing,
                volleys,
                attack_timer: &mut self.attack_timer,
                reload_worldtime: &mut self.reload_worldtime,
                reload: self.reload_stinger + (master.reload - self.reload_powerup),
                hit_chance: master.hit_chance,
                victim,
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
            let fired = strike.fire.map(|target| {
                let v = &*strike.victim;
                Aimed {
                    target,
                    position: v.position,
                    radius: [v.radius[0], v.radius[1], v.height],
                    class: v.class,
                    player: v.player,
                }
            });
            let (executed, own) = (strike.executed, std::mem::take(&mut strike.own));
            let posted = std::mem::take(&mut strike.posted);
            let (let_go, forgets) = (strike.let_go, strike.forgets);
            let sends_home = strike.sends_home;
            c.blasted = blasted;
            if let Some(t) = let_go {
                c.releases.push((t, self.releaser(c)));
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
            c.own_killed = self.hit_back(own)?;
            if matches!(code, 1 | 3) {
                self.delete_attack(c)?;
            }
        }
        Ok(())
    }
}

impl Unit {
    /// `RGE_Combat_Object::update`'s timers (`0x14107dfb0`, before the actions): the attack timer
    /// runs down, and a reload world time that passed ends the volley under way.
    pub(crate) fn run_down_attack_timers(&mut self, c: &Context<'_>) {
        // RGE_Combat_Object::update (0x14107dfb0): the attack timer runs down by dt times
        // `+0x87c` (a terrain look's rate, [`crate::combat::abilities`])
        if 0.0 < self.attack_timer {
            self.attack_timer -= c.dt_seconds * self.abilities.attack_rate;
            if self.attack_timer < 0.0 {
                self.attack_timer = 0.0;
            }
        }
        // then a reload world time past ends the volley under way (`+0xa04`) and is none
        if self.reload_worldtime != 0 && self.reload_worldtime < c.world_time {
            self.next_volley = 0;
            self.reload_worldtime = 0;
        }
    }

    /// The hunter or the attacker as a combat object, which its hits read and change: none for a
    /// unit without a Hunt or an Attack, or without its master.
    pub(crate) fn own_victim(&self, c: &Context<'_>) -> Option<Victim> {
        (self.actions.hunt().is_some() || self.actions.attack().is_some())
            .then_some(())
            .and_then(|()| {
                let master = c.masters?.get(self.life.master_id)?;
                let terrain = c.spatial.terrain.as_ref();
                Some(victim_of(
                    self,
                    master,
                    c.attributes,
                    c.player,
                    terrain,
                    c.mode_680,
                ))
            })
    }
}
