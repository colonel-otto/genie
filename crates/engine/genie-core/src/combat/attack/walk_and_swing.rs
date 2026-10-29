//! State 4's walk to the target and state 7's swing with its hit and its trample.

use crate::combat::attack::{
    AfterHit, Attack, AttackReads, DRAW_SCALE, EPSILON, FAILED, Fight, HALF, Hit, KILLED, MOVE,
    Readiness, after_hit, blast_so_far, reset_sprite, set_attack_timer, set_walker_anim,
    squared_reach, strike,
};
use crate::combat::damage::Victim;
use crate::combat::victims::write_strike_back;
use crate::combat::volley::Aimed;
use crate::math::rng::{Stream, Well512};
use crate::movement::move_to::{
    Follow, MoveTo, Shown, follow_path_end, start_walk_shown, target_moved, walk_or_give_up,
};
use crate::movement::{Me, PathAsk, footprint_gap_squared};
use crate::objects::lifecycle::seconds;
use crate::objects::sprite::{ActiveSprite, new_sprite};
use crate::objects::unit::Unit;
use crate::unit_ai::ACTION_EXECUTED;
use crate::world::step::Step;
use crate::{GenieError, combat::damage};

impl Attack {
    /// State 4 (cases 3, 4, 10): the `MoveTo` walks (`FUN_140a992b0`) to the target at `to`
    /// (its place and radii); a unit still farther than its reach goes on (code 0), one within
    /// it turns ready (`set_state` 6). The squared reach of a unit without a range is 0.01. A
    /// `MoveTo` that arrived returns 1: the list deletes it (its destructor, `0x140aa1f30`,
    /// stops the unit) and the unit turns ready (`set_state` 6).
    pub(super) fn walk(
        &mut self,
        c: &mut Step<'_>,
        unit: &mut Unit,
        me: Me,
        f: &Fight,
        r: &AttackReads,
        (target, radius): ([f32; 3], [f32; 2]),
    ) -> Result<u8, GenieError> {
        let code = match self.move_to {
            Some(MoveTo::Start) => {
                if !self.walk_start(c, unit, me, f, r)? {
                    return Ok(0);
                }
                0
            }
            Some(MoveTo::Moving) => {
                self.walk_on(c, unit, me, f, r)?;
                0
            }
            // case 2: the timer runs by dt; at its end the MoveTo asks again (set_state 3)
            Some(MoveTo::Waiting) => {
                self.move_wait.timer += seconds(c.dt);
                if self.move_wait.delay <= self.move_wait.timer {
                    self.move_wait.timer = 0.0;
                    self.move_to = Some(MoveTo::Start);
                }
                0
            }
            Some(MoveTo::Arrived) => {
                self.move_to = None;
                unit.body
                    .mover_mut()
                    .ok_or(crate::world::ATTACKER_UNMOVING)?
                    .stop()?;
                if self.state != MOVE {
                    return self.arrived_at_seen(c, unit, f, r, target);
                }
                self.ready(unit, &mut c.w.attack_ground(), f, r)?;
                return Ok(0);
            }
            None => {
                return Err(GenieError::unsupported(
                    "an Attack in state 4 without its MoveTo",
                ));
            }
            // case 3, the quiet MoveTo's failure (its update returns 3 without a notify):
            // set_state(13); a unit's own Attack (`+0x60` 0) posts 0x1f9 with 600; the update
            // returns 3 and the list removes the Attack
            Some(MoveTo::Failed) => {
                self.move_to = None;
                self.finish_in(FAILED, unit, c.w.streams.get(Stream::Sprite))?;
                if !self.sub_action {
                    unit.post_own(crate::unit_ai::ACTION_FAILED, [600, 0, 0])?;
                }
                return Ok(3);
            }
        };
        let ground = unit
            .body
            .mover()
            .map(|m| [m.stats.radius[0], m.stats.radius[1]])
            .ok_or(crate::world::ATTACKER_UNMOVING)?;
        let at = unit.body.position();
        let reach = squared_reach(self.range);
        if reach.abs() <= EPSILON || reach < footprint_gap_squared(at, ground, target, radius) {
            return Ok(code);
        }
        self.ready(unit, &mut c.w.attack_ground(), f, r)?;
        Ok(code)
    }

    /// State 4's `MoveTo` start (case 3): after a wait `+0x6c` goes from 1 to 2 as it asks again;
    /// it asks for its path and moves (`set_state` 4), which shows its sprite (the Attack's
    /// `+0x68`, [`Arms::move_sprite`]; slot 0x68 takes the master's standing one for none) before
    /// the motion step ([`start_walk_shown`]); the units in the way are posted at once. In range
    /// at once it arrives, showing the idle sprite with `+0x7b8` cleared. False when the motion
    /// step gave up ([`Attack::give_up`]; a `MoveTo` that fails at once shows the idle sprite
    /// too).
    ///
    /// [`Arms::move_sprite`]: crate::combat::attack::Arms::move_sprite
    pub(super) fn walk_start(
        &mut self,
        c: &mut Step<'_>,
        unit: &mut Unit,
        me: Me,
        f: &Fight,
        r: &AttackReads,
    ) -> Result<bool, GenieError> {
        // case 3 after a wait: `+0x6c` goes from 1 to 2 as it asks again
        if self.move_wait.state == 1 {
            self.move_wait.state = 2;
        }
        self.move_to = Some(MoveTo::Moving);
        let (goal, range) = self.goal();
        // the MoveTo's set_state(4) shows its sprite (`+0x58`, the Attack's `+0x68`)
        let graphic = self.arms.move_sprite.or(r.standing);
        let (walk, sprite_rng, _) = c.walk_sprites_action(&mut unit.body, me);
        let shown = Shown {
            sprite: &mut unit.sprite,
            rng: sprite_rng,
            graphic,
        };
        let ask = PathAsk::default();
        let walked = start_walk_shown(walk, (goal, range, false), ask, Some(shown))?;
        unit.post_in_the_way()?;
        let Some(state) = walked else {
            // the motion step gave up in the update set_state(4) ran; a MoveTo that fails
            // at once (set_state 13, not quiet: 0x140aa29e0) shows the idle sprite and
            // clears `+0x7b8` as an arrived one does
            self.give_up(r, f.own.class);
            if self.move_to == Some(MoveTo::Failed) {
                self.show_idle(unit, c.w.streams.get(Stream::Sprite))?;
            }
            return Ok(false);
        };
        if state == MoveTo::Arrived {
            // in range at its start: the MoveTo, not quiet, shows the idle sprite and
            // clears `+0x7b8`
            self.show_idle(unit, c.w.streams.get(Stream::Sprite))?;
        }
        self.move_to = Some(state);
        Ok(true)
    }

    /// The `MoveTo`'s idle sprite (slot 0x68 with the Attack's `+0x80`) with `+0x7b8` cleared, as
    /// a `MoveTo` that is not quiet shows it when it arrives or fails at its start.
    fn show_idle(&self, unit: &mut Unit, rng: &mut Well512) -> Result<(), GenieError> {
        let g = self
            .arms
            .idle_sprite
            .ok_or(crate::world::NO_STANDING_SPRITE)?;
        new_sprite(&mut unit.sprite, unit.body.move_stats_mut(), g, rng);
        set_walker_anim(&mut unit.body, 0.0);
        Ok(())
    }

    /// State 4's `MoveTo` walking (case 4): it takes the Attack's `+0x61` (`0x140a824ac`, which
    /// the Hunt's `set_state(12)` sets): a target that left its point asks for a path again (with
    /// `+0x61` only `set_state` 3; without it notify 0x1fc, `set_state` 3 and the motion step), a
    /// walker on its path's last stretch follows it; else the motion step alone. A motion step
    /// that gives up gives the Attack up ([`Attack::give_up`]); the units in the way are posted
    /// at once.
    pub(super) fn walk_on(
        &mut self,
        c: &mut Step<'_>,
        unit: &mut Unit,
        me: Me,
        f: &Fight,
        r: &AttackReads,
    ) -> Result<(), GenieError> {
        // a MoveTo to a point follows nothing
        let follow = if self.state == MOVE {
            target_moved(&mut c.walk(&mut unit.body, me), self.target, self.repaths)?
        } else {
            Follow::Stays
        };
        if let Follow::PathEnd(p) = follow {
            follow_path_end(&mut c.walk(&mut unit.body, me), p)?;
        }
        if follow.again() && self.repaths {
            // with `+0x61` the MoveTo only turns to state 3 (`LAB_140aa3a59`)
            self.move_to = Some(MoveTo::Start);
            return Ok(());
        }
        if follow.again() {
            // without it the MoveTo tells the unit (notify 0x1fc), turns to state 3 and runs
            // the motion step (`LAB_140aa320c`): moving on, it stays in state 3
            unit.post_own(0x1fc, [0; 3])?;
        }
        let walked = walk_or_give_up(c.walk(&mut unit.body, me))?;
        unit.post_in_the_way()?;
        match walked {
            Some(MoveTo::Moving) if follow.again() => self.move_to = Some(MoveTo::Start),
            Some(m) => self.move_to = Some(m),
            None => self.give_up(r, f.own.class),
        }
        Ok(())
    }

    /// State 7 (case 6). A melee swing counts down and lands its hit when its sprite reaches the
    /// firing frame (half its frames without one): the attack timer becomes the reload less the
    /// time to that frame (`FUN_141089820`) and the target takes the hit; a dead target ends the
    /// swing (`set_state` 2). A ranged unit's swing does not count down and fires at its firing
    /// frame (at once with frame 0): its volley at once ([`Unit::volley`]). After the hit, the
    /// swing's sprite looping makes the unit ready again (`set_state` 6).
    pub(super) fn swing(
        &mut self,
        c: &mut Step<'_>,
        unit: &mut Unit,
        f: &mut Fight,
        r: &AttackReads,
    ) -> Result<u8, GenieError> {
        let Some(sprite) = unit.sprite else {
            return Err(GenieError::unsupported("a swing without the unit's sprite"));
        };
        if self.pending {
            self.swing_to_hit(c, unit, f, r, sprite)?;
        } else {
            self.swing_after_hit(c, unit, f, r, sprite)?;
        }
        Ok(0)
    }

    /// The swing before its hit (state 7 with the hit pending): a melee swing counts down; at the
    /// firing frame (half the sprite's frames for a melee unit without one, at once with frame 0
    /// for a ranged one) the hit lands ([`Attack::land_hit`]). Class 51's swing, a melee swing of
    /// classes 35 and 64, and a ranged one without a firing frame are not modelled.
    pub(super) fn swing_to_hit(
        &mut self,
        c: &mut Step<'_>,
        unit: &mut Unit,
        f: &mut Fight,
        r: &AttackReads,
        sprite: ActiveSprite,
    ) -> Result<(), GenieError> {
        let ranged = self.arms.missile != -1;
        if f.own.class == 51 || (!ranged && matches!(f.own.class, 35 | 64)) {
            return Err(GenieError::unsupported("a swing of class 35, 51 or 64"));
        }
        if ranged && self.arms.fire_frame < 0 {
            return Err(GenieError::unsupported(
                "a ranged swing without a firing frame",
            ));
        }
        let fire = if ranged {
            self.arms.fire_frame
        } else if self.arms.fire_frame < 1 {
            (f32::from(sprite.graphic.frames) * HALF) as u32 as i16
        } else {
            self.arms.fire_frame
        };
        if !ranged {
            self.swing -= seconds(c.dt);
        }
        if fire != 0 && !sprite.reached(fire) {
            return Ok(());
        }
        self.land_hit(c, unit, f, r, sprite)
    }

    /// The swing's hit at its firing frame: the attack timer becomes the reload less the time to
    /// that frame (`FUN_141089820`); the volley fires at once ([`Unit::volley`]) for a ranged
    /// unit or one with volleys, else the target takes the melee hit ([`Attack::hit`]): the unit
    /// tells its unit AI the action executed (0x200 with 600) and the target and the blast's
    /// objects go back to the world there ([`write_strike_back`]). A dead target ends the swing
    /// (`set_state` 2); else the rest of the reload past the sprite is the swing's. A
    /// self-destructing attacker's hit is not modelled.
    pub(super) fn land_hit(
        &mut self,
        c: &mut Step<'_>,
        unit: &mut Unit,
        f: &mut Fight,
        r: &AttackReads,
        sprite: ActiveSprite,
    ) -> Result<(), GenieError> {
        let ranged = self.arms.missile != -1;
        let frame = if self.arms.fire_frame == 0 && !ranged {
            sprite.graphic.frames / 2
        } else {
            self.arms.fire_frame
        };
        set_attack_timer(
            unit,
            r.world_time,
            r.reload - f32::from(frame) * sprite.graphic.frame_duration,
        );
        // slot 0x858 (FUN_14120c0a0): the volley, but for a unit without a missile, volley
        // missile or charge projectile, whose hit is FUN_141081020's melee one
        if ranged || r.volleys {
            unit.volley(c, Aimed::at(self.target, &f.victim))?;
        } else {
            if matches!(f.own.master_id, 527 | 528 | 1104 | 2133..=2135) {
                return Err(GenieError::unsupported(
                    "a self-destructing attacker's hit (masters 527, 528, 1104, 2133..2135)",
                ));
            }
            let executed = {
                let object = c.w.streams.get(Stream::Object);
                Self::hit(f, &mut c.blasted, r.hit_chance, object)?
            };
            if let Some(target) = executed {
                unit.post_own(ACTION_EXECUTED, [600, target, 0])?;
            }
            write_strike_back(c, unit, &mut f.victim)?;
        }
        self.pending = false;
        if f.victim.hp <= 0.0 {
            return self.enter(KILLED, unit);
        }
        let rest = r.reload - f32::from(sprite.graphic.frames) * sprite.graphic.frame_duration;
        self.rest = rest;
        self.swing = rest;
        if rest < 0.0 {
            self.rest = 0.0;
        }
        Ok(())
    }

    /// The swing after its hit: a done swing ends (`set_state` 2), a ready one readies
    /// (`set_state` 6), one past its interrupt frame checks its target (readyToAttack with 0: one
    /// out of reach is followed, `set_state` 4). Once the sprite looped (or does not loop) the unit
    /// is ready again; a reload longer than the sprite waits at its last frame with the idle
    /// attack sprite (else the standing one) at speed 1.
    pub(super) fn swing_after_hit(
        &mut self,
        c: &mut Step<'_>,
        unit: &mut Unit,
        f: &Fight,
        r: &AttackReads,
        sprite: ActiveSprite,
    ) -> Result<(), GenieError> {
        let ranged = self.arms.missile != -1;
        let mut g = c.w.attack_ground();
        match after_hit(
            ranged,
            unit.attack_timer,
            self.target != -1,
            self.arms.interrupt_frame,
            self.arms.fire_frame,
            sprite.frame,
        ) {
            AfterHit::Done => return self.enter(KILLED, unit),
            AfterHit::Ready => return self.ready(unit, &mut g, f, r),
            // readyToAttack with its third argument 0: a target out of reach is followed
            // (`set_state` 4); a ready unit only faces it; the swing goes on either way
            AfterHit::Continue { check: true } => {
                if let Readiness::Away(gap) = self.ready_to_attack(unit, &mut g, f, r, false)? {
                    self.follow(unit, g.sprite_rng, f, gap)?;
                }
            }
            AfterHit::Continue { check: false } => {}
        }
        // the loop test reads the unit's sprite as it is now (`+0x140`): readyToAttack may have
        // shown the standing one
        let sprite = unit.sprite.unwrap_or(sprite);
        if !sprite.looped && sprite.graphic.flags & 1 != 0 {
            return Ok(());
        }
        // a reload longer than the sprite: at its last frame the unit waits with its idle attack
        // sprite (`+0x88`), else its standing one (slot 0x68), from its first frame when it
        // changes (`FUN_1413b9f00`), at speed 1
        if 0.0 < self.swing && sprite.reached(sprite.graphic.frames) {
            let idle = self
                .arms
                .idle_attack_sprite
                .or(self.arms.idle_sprite)
                .ok_or(GenieError::unsupported(
                    "a swing's end without a sprite to wait with",
                ))?;
            let changed = sprite.graphic.id != idle.id;
            new_sprite(
                &mut unit.sprite,
                unit.body.move_stats_mut(),
                idle,
                g.sprite_rng,
            );
            if changed && let Some(current) = unit.sprite.map(|sp| sp.graphic) {
                let _ = reset_sprite(&mut unit.sprite, current);
            }
            set_walker_anim(&mut unit.body, 1.0);
        }
        self.ready(unit, &mut g, f, r)
    }

    /// The hit of a melee unit (slot 0x858 -> `FUN_141081020` -> slot 0x860,
    /// `FUN_1412115e0`) of hit chance `chance` on the fight's target, the blast's objects
    /// `blasted` about it: a unit with an area effect and a weapon range below 0.99 tramples
    /// ([`Attack::trample`]) unless its charge of type 3 is not its hit's (slot 0x2f0); else
    /// `FUN_141084230` ([`Attack::strike`]). Returns the target of the executed action the unit
    /// tells its unit AI of (0x200 with 600). Not held: the attack count (`+0x900`) and the
    /// damaged flag of masters 83, 293, 122 and 216 (`+0x683`); masters 122 and 123 under the
    /// game's `+0xc14` (`FUN_140c884c0`, 0 as the tick oracle holds it) would take the target at
    /// once.
    pub(super) fn hit(
        f: &mut Fight,
        blasted: &mut [Victim],
        chance: i16,
        object_rng: &mut Well512,
    ) -> Result<Option<i32>, GenieError> {
        let a = f.own.source;
        if 0.0 < a.area_effect
            && a.weapon_range < damage::BLAST_RANGE
            && (a.charge_type != 3 || damage::charge_hit(&a, &f.victim))
        {
            return Self::trample(f, blasted, chance, object_rng);
        }
        Self::strike(f, blasted, chance, object_rng)
    }

    /// `FUN_141084230` with the unit's own place as the hit's origin ([`strike`]).
    pub(super) fn strike(
        f: &mut Fight,
        blasted: &mut [Victim],
        chance: i16,
        object_rng: &mut Well512,
    ) -> Result<Option<i32>, GenieError> {
        let hit = Hit {
            at: f.own.position,
            from_z: Some(f.own.position[2]),
            missile: false,
            chance,
        };
        let struck = strike(&mut f.own, &mut f.victim, blasted, hit, object_rng)?;
        Ok(struck.executed)
    }

    /// `FUN_1412115e0`'s trample: the blast about the unit, past its target ([`damage::blast`];
    /// the target stays among the objects on its tiles, the main building its annexes' hits
    /// go to); a building target's blast so far (`+0xb90`, its main building's for an annex,
    /// `0x141211933`) is what its hit leaves out; the unit tells its unit AI the action executed
    /// (0x200, whatever its master); the 64-bit Object-stream draw against the hit chance; a hit
    /// the target's shield does not block (`FUN_14108d8a0`, on a target of any type) is its slot
    /// 0xe8 at the elevation factor times the hit's 1. A charge of type 1 or 3 the blast or the
    /// hit was for (`+0x8a8`) is emptied.
    pub(super) fn trample(
        f: &mut Fight,
        blasted: &mut [Victim],
        chance: i16,
        object_rng: &mut Well512,
    ) -> Result<Option<i32>, GenieError> {
        let (center, target) = (f.own.position, f.victim.id);
        let mut targets: Vec<&mut Victim> = blasted.iter_mut().collect();
        targets.push(&mut f.victim);
        let _ = damage::blast(&mut f.own, center, Some(target), None, &mut targets, 1.0)?;
        let taken = blast_so_far(&f.victim, blasted)?;
        let draw = object_rng.raw64();
        let r = (i64::from(draw as u32 & 0x7fff_ffff) * 100) / DRAW_SCALE;
        if r <= i64::from(chance) {
            let factor =
                damage::elevation_factor(f.own.position[2], f.own.attack_elevation, &f.victim);
            if !damage::shield(&mut f.victim, false) {
                let weapons = f.own.weapons.clone();
                let mut others: Vec<&mut Victim> = blasted.iter_mut().collect();
                damage::take(
                    &mut f.victim,
                    &mut f.own,
                    &mut others,
                    &weapons,
                    factor,
                    taken,
                )?;
            }
        }
        let a = &mut f.own.source;
        if a.charge_hit && matches!(a.charge_type, 1 | 3) {
            a.charge = 0.0;
        }
        Ok(Some(target))
    }
}
