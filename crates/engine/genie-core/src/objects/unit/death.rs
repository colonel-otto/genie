//! A unit's explore, its stopped mover, the default task's fly and the deaths.

use crate::GenieError;
use crate::construction::build::{Completion, Construction};
use crate::math::rng::Stream;
use crate::movement::fly::Fly;
use crate::objects::lifecycle::Class;
use crate::objects::lifecycle::death_parts::{built_death, combat_dies, farm_bonus, refund_of};
use crate::objects::masters::Master;
use crate::objects::unit::{NO_DYING_SPRITE, Unit};
use crate::world::context::Context;

impl Unit {
    /// The unit's explore (slot 0x398, `FUN_141084db0` with its argument 4 set) to the point at
    /// its own height: its top Explore takes the point (slot 0x48), else a new Explore
    /// (`FUN_140a87d30`) becomes its only action (`set_only_action`, `FUN_14106e990`: the actions
    /// it had go, `FUN_140a98620`, and the new one starts, `FUN_140a977e0`). A `MoveTo` of the
    /// Explore that goes stops the unit (its teardown, slot 0x5d0); other actions it would
    /// replace are not modelled.
    pub(crate) fn explore_to(
        &mut self,
        point: [f32; 2],
        c: &mut Context<'_>,
    ) -> Result<(), GenieError> {
        let at = [point[0], point[1], self.position[2]];
        let owner = if let Some(e) = self.explore.as_mut() {
            let walked = e.move_to.is_some();
            let owner = e.take_point(at);
            if walked {
                self.stop_mover()?;
            }
            owner
        } else {
            if self.top_action().is_some() || self.unmodelled_action.is_some() {
                return Err(GenieError::unsupported(
                    "an Explore replacing another action",
                ));
            }
            let (e, owner) = crate::movement::explore::ExploreAction::new(at);
            self.explore = Some(e);
            owner
        };
        self.explore_owner(owner, c)
    }

    /// What an Explore's `set_state` asks of its unit ([`crate::movement::explore::Owner`]): its notify 0x202
    /// with the explore's action value to the unit AI, and the standing sprite (slot 0x68 with
    /// the master's `+0x470`) where the slice holds the unit's sprite. `Set_state` 2's slots are
    /// not modelled.
    pub(crate) fn explore_owner(
        &mut self,
        owner: crate::movement::explore::Owner,
        c: &mut Context<'_>,
    ) -> Result<(), GenieError> {
        use crate::movement::explore::Owner;
        match owner {
            Owner::Nothing => return Ok(()),
            Owner::Stopped => {
                return Err(GenieError::unsupported(
                    "an Explore's set_state 2 (slots 0x68)",
                ));
            }
            Owner::WantsPoint => self.post_own(
                crate::movement::explore::WANTS_POINT,
                [crate::movement::explore::EXPLORE_ACTION, 0, 0],
            )?,
            Owner::Standing => {}
        }
        if let Some(m) = self.mover.as_mut() {
            m.shows_standing();
        }
        if self.sprite.is_some() {
            let standing = c
                .masters
                .and_then(|m| m.get(self.life.master_id))
                .map(|m| m.standing_sprite)
                .and_then(|id| c.graphics.get(&id).copied())
                .ok_or(GenieError::unsupported(
                    "an explorer without its standing sprite",
                ))?;
            crate::combat::attack::set_sprite(
                &mut self.sprite,
                standing,
                c.streams.get(Stream::Sprite),
            );
        }
        Ok(())
    }

    /// Slot 0x5d0 of a unit whose `MoveTo` went.
    pub(crate) fn stop_mover(&mut self) -> Result<(), GenieError> {
        self.mover
            .as_mut()
            .ok_or(crate::world::WALKER_UNMOVING)?
            .stop()
    }

    /// A stop since the last look cleared the animation speed (`+0x7b8`, [`Mover::speed_stopped`]).
    ///
    /// [`Mover::speed_stopped`]: crate::movement::Mover::speed_stopped
    pub(crate) fn take_stopped_speed(&mut self) {
        if let Some(m) = self.mover.as_mut().filter(|m| m.speed_stopped) {
            m.speed_stopped = false;
            self.anim_speed = Some(0.0);
        }
    }

    /// The default task's Fly ([`crate::movement::fly::Fly::start`]): a player's bird (its task's
    /// `+0x15`, not held) is not modelled; a master of fly mode 1 lifts the bird to the fly
    /// height (slot 0x58, the position setter: its tile stays, so do its counts).
    pub(crate) fn start_fly(
        &mut self,
        c: &mut Context<'_>,
        task: crate::objects::masters::Task,
    ) -> Result<(), GenieError> {
        if c.player != 0 {
            return Err(GenieError::unsupported(
                "a player's bird's default Fly (its task's +0x15)",
            ));
        }
        let fly_mode = c
            .masters
            .and_then(|m| m.get(self.life.master_id))
            .map(|m| m.fly_mode)
            .ok_or(GenieError::unsupported(
                "a bird of a master the slice does not have",
            ))?;
        self.fly = Some(Fly::start(
            task,
            self.position,
            c.streams.get(Stream::Action),
        ));
        if fly_mode == 1 {
            self.position[2] = crate::movement::fly::HEIGHT;
            c.spatial.set_position(self.id, self.position);
            // the position setter marks the move (`+0x824`, `+0x870`)
            if let Some(m) = self.mover.as_mut() {
                m.mark_move(self.position, c.world_time);
            }
        }
        Ok(())
    }

    /// The death's `set_state(3)` (from `before`, the state the object had) and the static
    /// update's dying sprite after it (slot 0x7a0, then slot 0x68, `FUN_141217000`). A building's
    /// (`FUN_1411dbb70`) first sets its annexes' states (the caller's after the update), then a
    /// built one dies ([`built_death`]) and a foundation runs the completion's first part and its
    /// construction's start ([`Unit::foundation_died`]); then the combat object's
    /// (`FUN_14121cc40`, [`combat_dies`]), whose refund from state 2 ([`refund_of`]) the caller
    /// takes from its owner's attributes; then the base one's (`FUN_14119d710`, whose storage and
    /// sight are the caller's). An animated or moving object (types 20 to 69) shows its
    /// master's dying sprite. Not held: the particles and the sounds.
    pub(crate) fn died(&mut self, c: &mut Context<'_>, before: u8) -> Result<(), GenieError> {
        let Some(master) = c.masters.and_then(|m| m.get(self.life.master_id)) else {
            if self.life.dying_sprite.is_some() {
                return Err(GenieError::unsupported(
                    "a dying object of a master the slice does not have",
                ));
            }
            return Ok(());
        };
        if master.kind == 80 {
            // the static update's death (0x14118ad50) lets the annexes go before its set_state(3):
            // each slot (`+0xbc0`..) clears and its annex's `+0xbb8` with it, and each annex takes
            // the building's hit points (`+0x278`), so it dies in its own update; the set_state
            // (FUN_1411dbb70) then finds no annex to take along
            for annex in c.spatial.annexes_of(self.id) {
                c.spatial.set_annex(annex, None);
                c.released_annexes.push((annex, self.life.hp));
            }
            if before == 2 {
                let death = built_death(self, master, c.computer)?;
                c.attribute_adds.extend(death.adds);
                c.unwatched.push(self.id);
                if let (Some(v), Some(at)) = (c.spatial.sight.as_mut(), death.unlink) {
                    v.unlink_building(at, master.radius);
                }
                c.building_death = Some(death.building);
            } else if before == 0 {
                self.foundation_died(c, master)?;
            }
        } else if master.kind != 70 {
            // a static object's dying sprite is the lifecycle's ([`Lifecycle::die`]); an
            // animated or moving one shows its master's (slot 0x7a0, `FUN_1411887e0`: no
            // terrain look) through slot 0x68 (a moving one's `FUN_141158620` derives its facet
            // from its angle again)
            if self.life.class == Class::Animated
                && self.life.dying_sprite.is_some()
                && let Some(id) = crate::objects::sprite::dying(master, None)
            {
                let g = c.graphics.get(&id).copied().ok_or(NO_DYING_SPRITE)?;
                crate::combat::attack::set_sprite(
                    &mut self.sprite,
                    g,
                    c.streams.get(Stream::Sprite),
                );
            }
            return Ok(());
        }
        combat_dies(self, master, c.spatial, c.pathing)?;
        if before == 2 {
            c.refund = refund_of(master, self.annex_of);
        }
        // the static update's sprite after the death (0x14118ad50): a building's by its build
        // points (its destruction sprite unbuilt), any other combat object's slot 0x7a0's
        let look = self.abilities.look.as_ref();
        let shown = if master.kind == 80 {
            let built = self
                .construction
                .ok_or(crate::construction::build::NO_CONSTRUCTION)?;
            crate::objects::sprite::building_death(master, look, built.progress)
        } else {
            crate::objects::sprite::dying(master, look)
        };
        if let Some(id) = shown {
            let g = c.graphics.get(&id).copied().ok_or(NO_DYING_SPRITE)?;
            crate::combat::attack::set_sprite(&mut self.sprite, g, c.streams.get(Stream::Sprite));
        }
        Ok(())
    }

    /// A foundation's death (`FUN_1411dbb70` to state 3 from state 0), before the combat
    /// object's: the completion's first part (`FUN_1411c6bd0`, [`crate::construction::build::completion_part`]:
    /// a farm's bonus once its build points reached its build time, a mule cart's search for
    /// resources, the caller's), then, as the state is still 0 and 3 lies within 1 to 6, its
    /// construction's start (`FUN_1411e5670`, [`Construction::begin`]: a desolid one gets its
    /// record and sight, its missiles per volley its master's; its owner watches it, the
    /// caller's).
    pub(crate) fn foundation_died(
        &mut self,
        c: &mut Context<'_>,
        master: &Master,
    ) -> Result<(), GenieError> {
        let construction = self
            .construction
            .as_mut()
            .ok_or(crate::construction::build::NO_CONSTRUCTION)?;
        let completion = Completion {
            master: Some(master),
            annexed: false,
            attribute_204: false,
            mode_680: c.mode_680,
            farm_bonus: farm_bonus(c.attributes),
            masters: c.masters,
            computer: c.computer,
            ai: self.ai.is_some(),
            main: self.annex_of.is_some(),
            now: c.world_time,
            rice: self.rice,
        };
        let part = completion.first_part(construction)?;
        // its tracker part (FUN_14104eac0): the foundation leaves the pending lists
        let k = crate::players::tracker::key(c.masters, c.lines, self.life.master_id);
        c.tracker.complete(self.id, k);
        c.attribute_adds.extend(part.attributes);
        if part.pasture {
            c.pastures.push(self.id);
        }
        if part.cart {
            c.carts.push((self.id, 0));
        }
        let started = Construction::begin(
            self.id,
            construction,
            self.mover.as_mut(),
            c.spatial,
            c.pathing,
            c.world_time,
        )?;
        if started {
            construction.started = false;
            c.started.push(self.id);
            self.firepower = master.volley_fire;
        }
        Ok(())
    }
}
