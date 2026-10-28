//! The hunt's strike as the hunter's update runs it: the hunter as a gatherer and an attacker,
//! the herd it leads, the hunt's update and what follows.

use crate::GenieError;
use crate::combat::attack::Strike;
use crate::combat::damage::Victim;
use crate::combat::victims::{KillLeft, victim, write_struck};
use crate::combat::volley::Aimed;
use crate::gathering::drop_sites::nearest_drop_site;
use crate::gathering::hunt::{Gatherer, Herd, HuntAction, HuntSprites};
use crate::movement::move_to::MoveTo;
use crate::movement::{Me, footprint_gap_squared};
use crate::objects::masters::Master;
use crate::objects::unit::Unit;
use crate::unit_ai::ACTION_EXECUTED;
use crate::world::TREE_NEAR;
use crate::world::context::Context;
use crate::world::others::{Others, OthersRef};

impl Unit {
    /// The hunt's strike in the hunter's update (`0x140ac7a00`'s states 4, 6, 7, 10 and 12): what
    /// the hunt reads of the hunter as a gatherer and an attacker, of its target and of the herd
    /// it leads, the hunt's update, and what follows: the volley, the notifies, the hit back, the
    /// drop-off, the search, the targets let go and an attack that failed (`own`: the hunter as
    /// a combat object, taken).
    pub(crate) fn hunt_strike(
        &mut self,
        c: &mut Context<'_>,
        others: &mut Others<'_>,
        me: Me,
        own: &mut Option<Victim>,
        tree_lost: bool,
    ) -> Result<(), GenieError> {
        let computer = c.computer;
        let master = c
            .masters
            .and_then(|m| m.get(self.life.master_id))
            .ok_or(crate::world::HUNTER_MASTER)?;
        let action = self.actions.hunt().copied().expect("the caller's test");
        let mut victim = hunt_victim(c, others, &action)?;
        let own = own.take().ok_or(crate::world::HUNTER_MASTER)?;
        let herd = self.herd_for(c, others.view(), &action, master, &victim)?;
        let hunt = self.actions.hunt_mut().expect("the caller's test");
        // the gatherer's view: the master's work rate (+0x798) plus the powerup (slot 0x6e8)
        let productivity = usize::try_from(hunt.task.productivity)
            .ok()
            .and_then(|i| c.attributes.get(i))
            .copied()
            .filter(|&v| 0.0 < v)
            .unwrap_or(1.0);
        let g = |id: i16| (0 <= id).then(|| c.graphics.get(&id).copied()).flatten();
        let sprites = HuntSprites {
            moving: g(hunt.task.sprites[0]),
            working: g(hunt.task.sprites[2]),
            carrying: g(hunt.task.sprites[3]),
            standing: g(master.standing_sprite),
        };
        let mut gather = Gatherer {
            held_kind: &mut self.life.attribute_type,
            held: &mut self.life.amount,
            work_rate: self.work_rate_powerup + master.work_rate,
            capacity: f32::from(master.capacity),
            productivity,
            target_class: victim.class,
            target_storage: victim.storage,
            stops_decay: c.attributes.get(292) == Some(&1.0),
            ai_class: self.ai.as_ref().map(|ai| ai.target_type),
            sprites,
            tree_searched: tree_lost,
            predator_hunts_end: c.predator_hunts_end,
            searches: false,
            leads: None,
            attack_failed: false,
            let_go: None,
        };
        let mut blasted = std::mem::take(&mut c.blasted);
        // the unit AI's canMoveToAttack (FUN_140c48420) and stance
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
        let swing_sprites =
            crate::combat::attack::swing_sprites(self.abilities.look.as_ref(), master, c.graphics);
        let (walk, sprite_rng) = c.walk_and_sprites(self.body.mover_mut(), me);
        let mut strike = Strike {
            sprite: &mut self.sprite,
            standing,
            volleys,
            attack_timer: &mut self.attack_timer,
            reload_worldtime: &mut self.reload_worldtime,
            reload: self.reload_stinger + (master.reload - self.reload_powerup),
            hit_chance: master.hit_chance,
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
            armed: true,
            volley_under_way: self.next_volley != 0,
            let_go: None,
            forgets: false,
            sends_home: false,
            swing_sprites,
        };
        let full = hunt.update(walk, computer, &mut strike, &mut gather, herd)?;
        let searches = gather.searches;
        let (attack_failed, let_go) = (gather.attack_failed, gather.let_go);
        let posted = std::mem::take(&mut strike.posted);
        let leads = gather.leads.map(|site| (hunt.target, site));
        let fired = strike.fire.map(|target| Aimed::at(target, strike.victim));
        let (executed, own) = (strike.executed, std::mem::take(&mut strike.own));
        let struck = write_struck(c, others, blasted, &mut victim)?;
        if let Some(aimed) = fired {
            self.volley(c, aimed)?;
        }
        if let Some(target) = executed {
            self.post_own(ACTION_EXECUTED, [600, target, 0])?;
        }
        for (kind, p) in posted {
            self.post_own(kind, p)?;
        }
        struck.file(c, self.hit_back(own)?.map(|k| KillLeft::own(self.id, k)));
        if full {
            self.hunt_drop_off(c, others.view())?;
        }
        if searches {
            self.hunt_set_search(c, others.view())?;
        }
        if let Some(target) = let_go {
            self.hunt_release(c, [target]);
        }
        if attack_failed {
            self.hunt_attack_failed(c, others.view())?;
        }
        c.leads = leads;
        Ok(())
    }

    /// The hunt's end of an Attack that failed (`FUN_140ac4670` with -1 and 5): a hunter whose
    /// unit AI targets trees (its class, else the hunt's, that the civilian's slot 0x1b0 gives
    /// 0) takes the next tree within 5 of it ([`Unit::hunt_tree_search`]); any other, or one
    /// that finds none, searches (`set_state(3)` with the target kept,
    /// [`Unit::hunt_search_state`]).
    pub(crate) fn hunt_attack_failed(
        &mut self,
        c: &mut Context<'_>,
        others: OthersRef<'_>,
    ) -> Result<(), GenieError> {
        let tree = self.ai.as_ref().is_some_and(|ai| {
            let class = if ai.target_type == -1 {
                self.actions
                    .hunt()
                    .map_or(-1, |h| i32::from(h.target_class))
            } else {
                ai.target_type
            };
            class == crate::gathering::hunt::TREE_CLASS
        });
        if tree && self.hunt_tree_search(c, others, None, Some(TREE_NEAR))? {
            return Ok(());
        }
        let hunt = self.actions.hunt_mut().expect("the caller's hunt");
        hunt.tear_down(self.body.mover_mut())?;
        hunt.state = crate::gathering::hunt::SEARCH;
        self.hunt_search_state(c, others)
    }

    /// What `set_state(6)` on live livestock reads (`0x140ac60e0` case 6), and the lead's update
    /// (state 10, `0x140ac7a00` case 9: whatever the target, a dead one too) ([`Herd`]): the
    /// hunter's engagement and arms, the animal's owner, its drop site with the gap to it, the
    /// animal's idleness and the hunter's special graphic; none in the other states.
    fn herd_for(
        &self,
        c: &Context<'_>,
        others: OthersRef<'_>,
        action: &HuntAction,
        master: &Master,
        victim: &Victim,
    ) -> Result<Option<Herd>, GenieError> {
        let faces =
            action.state == crate::gathering::hunt::WALK && action.move_to == Some(MoveTo::Arrived);
        if !((faces && victim.class == crate::gathering::hunt::LIVESTOCK && victim.state <= 2)
            || action.state == crate::gathering::hunt::LEAD)
        {
            return Ok(None);
        }
        let at = |id| {
            c.spatial
                .get_known(id)?
                .ok_or(GenieError::unsupported("a hunt's object without a place"))
        };
        let (me_at, them) = (at(self.id)?, at(action.target)?);
        let speed = self.body.mover().map_or(0.0, |m| m.stats.speed);
        let engagement = crate::combat::attack::engagement(
            self.ai.as_ref(),
            speed,
            master,
            me_at,
            them,
            c.world_time,
        )?;
        let moving = action
            .task
            .walk_sprite_id(self.life.attribute_type, self.life.amount);
        let arms = crate::combat::attack::arms(master, c.graphics, action.task.sprites, moving);
        let own = them.player == c.player;
        let drop_site = if !own {
            None
        } else if action.drop_site != -1 {
            let site = c.spatial.get(action.drop_site);
            site.map(|s| {
                let d = footprint_gap_squared(them.position, them.radius, s.position, s.radius);
                (action.drop_site, d)
            })
        } else {
            if c.computer {
                return Err(GenieError::unsupported(
                    "a computer player's hunt of its livestock (its drop-site search)",
                ));
            }
            let sites: Vec<i16> = c
                .drop_sites
                .iter()
                .filter(|d| d.livestock)
                .map(|d| d.building)
                .collect();
            nearest_drop_site(others, &sites, them.position)
                .map(|(id, _)| -> Result<(i32, f32), GenieError> {
                    let s = at(id)?;
                    Ok((
                        id,
                        footprint_gap_squared(them.position, them.radius, s.position, s.radius),
                    ))
                })
                .transpose()?
        };
        // the animal's top action without a target or a point (slots 0x190, 0x288)
        let animal = others.get(action.target);
        let idle = match animal {
            Some(u) => u.top_action_idle()?,
            None => false,
        };
        Ok(Some(Herd {
            own,
            drop_site,
            engagement,
            arms,
            idle,
            special_graphic: if master.kind == 70 && master.special_graphic {
                Some(c.graphics.get(&master.special_sprite).copied().ok_or(
                    GenieError::unsupported("a special graphic the data file does not have"),
                )?)
            } else {
                None
            },
        }))
    }
}

/// The object the hunt's strike reads through the update's view ([`victim`]): its target, else
/// its Attack's. A hunt whose start dropped its target (gone or past state 6) and holds no
/// Attack reads none (state 7's end without a target): a stand-in no object holds.
fn hunt_victim(
    c: &Context<'_>,
    others: &Others<'_>,
    action: &HuntAction,
) -> Result<Victim, GenieError> {
    let target = match action.target {
        -1 => action.attack.map_or(-1, |a| a.target),
        t => t,
    };
    match target {
        -1 => Ok(Victim {
            id: -1,
            ..Victim::default()
        }),
        t => victim(c, others, t).ok_or(GenieError::unsupported(
            "an attack's target the slice does not hold live",
        )),
    }
}
