//! The hunter's part of its update: the hunt's targets checked, a lost tree, and the state the
//! hunt is in (the drop-off's wait and walk, the failure, the walks to a point, the search, the
//! idle, the lead).

use crate::GenieError;
use crate::combat::damage::Victim;
use crate::gathering::gather::unit::give_load;
use crate::movement::move_to::MoveTo;
use crate::movement::{Me, footprint_gap_squared};
use crate::objects::release::release_own;
use crate::objects::unit::{Unit, hunt_walk_sprite, second_target_checked};
use crate::unit_ai::task_finder::Actor;
use crate::unit_ai::task_finder::world::state_of;
use crate::world::context::Context;
use crate::world::{SEARCH_AGAIN, TREE_SEARCH_WITHIN};

impl Unit {
    /// The Hunt's update (`0x140ac7a00`) first checks its targets: one gone from the world's
    /// table is dropped (slot 0x68 with none), one past state 6 is let go (slot 0x180, its
    /// `+0x60` being 0) and dropped; then the second target ([`second_target_checked`]).
    pub(crate) fn hunt_targets(
        &mut self,
        c: &mut Context<'_>,
        others: &[Self],
    ) -> Result<(), GenieError> {
        if let Some(target) = self.hunt.map(|h| h.target).filter(|&t| t != -1) {
            let state = c
                .victim
                .as_deref()
                .filter(|v| v.id == target)
                .map(|v| v.state);
            let gone = match state {
                Some(s) => 6 < s,
                None if c.spatial.get(target).is_some() => {
                    return Err(GenieError::unsupported(
                        "a hunt's target the slice cannot read",
                    ));
                }
                None => true,
            };
            if gone {
                if state.is_some() {
                    c.releases.push((target, self.releaser(c)));
                }
                if let Some(h) = self.hunt.as_mut() {
                    h.set_target(-1);
                }
            }
        }
        if let Some(site) = self.hunt.map(|h| h.carry_to) {
            let held = c.spatial.get(site).is_some();
            let (kept, let_go) =
                second_target_checked(site, held, state_of(site, others, c.foreign));
            if let Some(h) = self.hunt.as_mut() {
                h.carry_to = kept;
            }
            if let Some(t) = let_go {
                self.hunt_release(c, [t]);
            }
        }
        Ok(())
    }

    /// A hunt whose target went on the walk to it (state 4, its `MoveTo` quiet: code 5), or
    /// whose `MoveTo` failed (code 3), and whose class is a tree's: it searches for the next tree
    /// (`FUN_140ac4670` with 8 and -1 both times).
    pub(crate) fn tree_lost(&self) -> bool {
        self.hunt.as_ref().is_some_and(|h| {
            h.state == crate::gathering::hunt::WALK
                && 0.0 <= h.timer
                && ((h.target == -1 && h.move_to.is_some()) || h.move_to == Some(MoveTo::Failed))
        }) && self.ai.as_ref().is_some_and(|ai| {
            let class = if ai.target_type == -1 {
                self.hunt.map_or(-1, |h| i32::from(h.target_class))
            } else {
                ai.target_type
            };
            class == crate::gathering::hunt::TREE_CLASS
        })
    }

    /// The Hunt's part of the hunter's update after its targets' check, by the state the hunt is
    /// in, in the update's order of its tests: the drop-off's wait ([`Unit::hunt_drop_wait`]) and
    /// walk ([`Unit::hunt_drop_walk`]), the failure ([`Unit::hunt_failed`]), the walks to a point
    /// ([`Unit::hunt_point_walk`]), the search ([`Unit::hunt_search_wait`]), the idle
    /// ([`Unit::hunt_idle`]), the wait after a given-up `MoveTo` ([`Unit::hunt_wait_walk`]), a
    /// lead without its target, a lost tree's search, else its strike ([`Unit::hunt_strike`]).
    /// False when the unit holds no hunt: its Attack's part runs then.
    pub(crate) fn hunt_part(
        &mut self,
        c: &mut Context<'_>,
        others: &mut [Self],
        me: Me,
        own: &mut Option<Victim>,
        tree_lost: bool,
    ) -> Result<bool, GenieError> {
        use crate::gathering::hunt::{
            DROP, FAILED, IDLE, LEAD, SEARCH, WALK, WALK_POINT, WALK_SEARCH,
        };
        let (state, timer, target) = self
            .hunt
            .as_ref()
            .map_or((None, 0.0, -1), |h| (Some(h.state), h.timer, h.target));
        match state {
            Some(DROP) if timer < 0.0 => self.hunt_drop_wait(c, others)?,
            Some(DROP) => self.hunt_drop_walk(c, others, me)?,
            Some(FAILED) => self.hunt_failed(c, others)?,
            Some(WALK_POINT | WALK_SEARCH) => self.hunt_point_walk(c, others, me)?,
            Some(SEARCH) => self.hunt_search_wait(c, others)?,
            Some(IDLE) => self.hunt_idle()?,
            Some(WALK) if target == -1 && timer < 0.0 => self.hunt_wait_walk(c)?,
            Some(LEAD) if target == -1 => {
                // state 10 without its target (0x140ac7a00 case 9): set_state(3), the search
                self.hunt.as_mut().expect("above").state = SEARCH;
                self.hunt_set_search(c, others)?;
            }
            // the hunt took the next tree (slot 0x48): its MoveTo's code 5 ends the update
            _ if tree_lost
                && self.hunt_tree_search(c, others, Some(TREE_SEARCH_WITHIN), None)? => {}
            Some(_) => self.hunt_strike(c, others, me, own, tree_lost)?,
            None => return Ok(false),
        }
        Ok(true)
    }

    /// Case 7 waiting to try again (state 8 with its timer below 0): the timer runs by dt; at its
    /// end the drop site goes (`FUN_140a7a190` with none: slot 0x180) and `set_state(8)` searches
    /// again ([`Unit::hunt_drop_off`]).
    pub(crate) fn hunt_drop_wait(
        &mut self,
        c: &mut Context<'_>,
        others: &[Self],
    ) -> Result<(), GenieError> {
        let hunt = self.hunt.as_mut().expect("the caller's hunt");
        hunt.timer += c.dt_seconds;
        if 0.0 <= hunt.timer {
            hunt.timer = 0.0;
            let site = std::mem::replace(&mut hunt.carry_to, -1);
            self.hunt_release(c, [site]);
            self.hunt_drop_off(c, others)?;
        }
        Ok(())
    }

    /// State 8, which has no target: its drop site (the second target) is what it walks to
    /// ([`crate::gathering::hunt::HuntAction::update_drop`]); a load dropped goes to the player
    /// ([`give_load`]) and the hunt walks back ([`Unit::hunt_after_drop`]); a drop site that does
    /// not take it goes and the drop-off searches again. Its idling (`set_state` 2) is not
    /// modelled.
    pub(crate) fn hunt_drop_walk(
        &mut self,
        c: &mut Context<'_>,
        others: &mut [Self],
        me: Me,
    ) -> Result<(), GenieError> {
        let hunt = self.hunt.as_mut().expect("the caller's hunt");
        let master = c
            .masters
            .and_then(|m| m.get(self.life.master_id))
            .ok_or(crate::world::HUNTER_MASTER)?;
        let me_at = c
            .spatial
            .get_known(self.id)?
            .ok_or(GenieError::unsupported("a hunter without a place"))?;
        let (me_position, me_radius) = (me_at.position, me_at.radius);
        let site = match c.spatial.get_known(hunt.carry_to)? {
            Some(s) => {
                let actor = Actor {
                    id: self.id,
                    master,
                    player: c.player,
                    held: (self.life.attribute_type, self.life.amount),
                    attribute_95: c.attributes.get(95).copied().unwrap_or(0.0),
                    civ: c.civ,
                };
                Some(crate::gathering::hunt::DropOff {
                    takes: crate::unit_ai::task_finder::drops_at(c.drop_sites, s.master, &actor),
                    gap_squared: footprint_gap_squared(
                        me_position,
                        me_radius,
                        s.position,
                        s.radius,
                    ),
                })
            }
            None => None,
        };
        let attributes = c.attributes.len();
        let shows = hunt_walk_sprite(hunt, c.graphics, &self.life);
        let who = crate::gathering::hunt::PointWalker {
            computer: c.computer,
            villager: master.class == 4,
            stance: self.ai.as_ref().map_or(0, |ai| ai.stance),
        };
        let (walk, rng) = c.walk_and_sprites(self.mover.as_mut(), me);
        let shown = crate::gathering::hunt::Shown {
            sprite: &mut self.sprite,
            rng,
            graphic: shows,
        };
        match hunt.update_drop(
            walk,
            &mut self.life.attribute_type,
            &mut self.life.amount,
            attributes,
            site,
            shown,
            who,
            &mut self.anim_speed,
        )? {
            crate::gathering::hunt::Dropped::Going => {}
            crate::gathering::hunt::Dropped::Load(kind, amount) => {
                // the player's slot 0xf8 (FUN_141616820) with 1 and 1: the resource, and its
                // gathered total (166 to 168; gold's 49 and 169); the statistics (slot 0xb8) and
                // the local player's tracker (FUN_14097daf0) are not held; a sharing player's
                // allies' share (+0x1426) is not modelled
                give_load(
                    (&c.spatial.relations, c.sharing),
                    c.player,
                    &mut c.attribute_adds,
                    kind,
                    amount,
                )?;
                self.hunt_after_drop(c, others)?;
            }
            crate::gathering::hunt::Dropped::Again => {
                // the drop site goes (FUN_140a7a190 with none), then set_state(8)
                let hunt = self.hunt.as_mut().expect("above");
                let site = std::mem::replace(&mut hunt.carry_to, -1);
                self.hunt_release(c, [site]);
                self.hunt_drop_off(c, others)?;
            }
            crate::gathering::hunt::Dropped::Idle => {
                return Err(GenieError::unsupported(
                    "a hunt's drop-off idling (set_state 2)",
                ));
            }
        }
        Ok(())
    }

    /// State 13 (`0x140ac7a00` case 0xc): notify 0x1f9 with the hunt's action value, and the
    /// update returns 3: the action list removes the hunt (`FUN_140a98e20`), whose teardown lets
    /// its target, then its second target go. One with a sub-action is not modelled.
    pub(crate) fn hunt_failed(
        &mut self,
        c: &mut Context<'_>,
        others: &mut [Self],
    ) -> Result<(), GenieError> {
        let hunt = self.hunt.as_ref().expect("the caller's hunt");
        let (target, site) = (hunt.target, hunt.carry_to);
        if hunt.attack.is_some() || hunt.move_to.is_some() {
            return Err(GenieError::unsupported("a failed hunt with a sub-action"));
        }
        self.post_own(
            crate::unit_ai::ACTION_FAILED,
            [crate::unit_ai::HUNT_ACTION, 0, 0],
        )?;
        self.hunt = None;
        if target != -1 {
            if others.iter().any(|u| u.id == target) {
                let by = self.releaser(c);
                release_own(others, c.masters, target, by)?;
            } else {
                // another player's (Gaia's deer, a neighbour's sheep): its slot 0x180 after this
                // update ([`World::release`]), as the other teardowns' releases
                self.hunt_release(c, [target]);
            }
        }
        self.hunt_release(c, [site]);
        Ok(())
    }

    /// States 15 and 11: the walk to a point
    /// ([`crate::gathering::hunt::HuntAction::update_point_walk`]); state 15's end is
    /// `set_state(2)`: the unit takes the hunt's sprite to stand with (slot 0x68 with
    /// `FUN_140ac5210`'s), the target and the second target go; state 11's is `set_state(3)`, the
    /// search ([`Unit::hunt_search_state`]).
    pub(crate) fn hunt_point_walk(
        &mut self,
        c: &mut Context<'_>,
        others: &[Self],
        me: Me,
    ) -> Result<(), GenieError> {
        let hunt = self.hunt.as_mut().expect("the caller's hunt");
        let master = c
            .masters
            .and_then(|m| m.get(self.life.master_id))
            .ok_or(crate::world::HUNTER_MASTER)?;
        let who = crate::gathering::hunt::PointWalker {
            computer: c.computer,
            villager: master.class == 4,
            stance: self.ai.as_ref().map_or(0, |ai| ai.stance),
        };
        let shows = hunt_walk_sprite(hunt, c.graphics, &self.life);
        let (walk, rng) = c.walk_and_sprites(self.mover.as_mut(), me);
        let shown = crate::gathering::hunt::Shown {
            sprite: &mut self.sprite,
            rng,
            graphic: shows,
        };
        match hunt.update_point_walk(walk, who, shown)? {
            crate::gathering::hunt::PointWalk::Walking => {}
            crate::gathering::hunt::PointWalk::Search => self.hunt_search_state(c, others)?,
            crate::gathering::hunt::PointWalk::Done { target, second } => {
                let task = hunt.task;
                self.hunt_stand(c, &task)?;
                self.hunt_release(c, [target, second]);
            }
        }
        Ok(())
    }

    /// State 3 (case 2): the timer runs by dt; at its end it starts again at −10 s and the unit
    /// AI is told again (notify 0x202, [`Unit::hunt_search_notify`]).
    pub(crate) fn hunt_search_wait(
        &mut self,
        c: &mut Context<'_>,
        others: &[Self],
    ) -> Result<(), GenieError> {
        let hunt = self.hunt.as_mut().expect("the caller's hunt");
        if hunt.timer < 0.0 {
            hunt.timer += c.dt_seconds;
            if 0.0 <= hunt.timer {
                hunt.timer = SEARCH_AGAIN;
                self.hunt_search_notify(c, others)?;
            }
        }
        Ok(())
    }

    /// State 2 (case 1): a done hunt waits; one of a tree's target class would search again
    /// (`set_state` 3), which is not modelled.
    pub(crate) fn hunt_idle(&self) -> Result<(), GenieError> {
        if self.hunt.as_ref().is_some_and(|h| h.target_class == 15) {
            return Err(GenieError::unsupported(
                "a done hunt of a tree's class (set_state 3)",
            ));
        }
        Ok(())
    }

    /// Case 3 waiting after a given-up `MoveTo`, its target gone: the timer runs by dt, then the
    /// walk to the point (`set_state` 11, [`crate::gathering::hunt::HuntAction::wait_then_walk`]).
    pub(crate) fn hunt_wait_walk(&mut self, c: &Context<'_>) -> Result<(), GenieError> {
        let hunt = self.hunt.as_mut().expect("the caller's hunt");
        let mover = self.mover.as_mut().ok_or(crate::world::HUNTER_UNMOVING)?;
        hunt.wait_then_walk(c.dt_seconds, mover)
    }
}
