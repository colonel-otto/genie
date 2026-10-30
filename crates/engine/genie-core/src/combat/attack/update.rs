//! One update (slot 6, `0x140a82ae0`): a target gone, a disarmed building, the walks to where
//! the target was seen.

use crate::GenieError;
use crate::combat::attack::walks::QUIET_CODE;
use crate::combat::attack::{
    AiOrder, Attack, AttackReads, Fight, GOTO_POINT, GOTO_SEEN, KILLED, MOVE, NO_RANGE, NONE,
    READY, Readiness, SEEN_RANGE, SWING, TOUCH, Walker, set_walker_anim,
};
use crate::math::rng::{Stream, Well512};
use crate::movement::move_to::GiveUp;
use crate::movement::{Me, footprint_gap};
use crate::objects::layers::Body;
use crate::objects::lifecycle::seconds;
use crate::objects::masters::class;
use crate::objects::sprite::ActiveSprite;
use crate::objects::sprite::new_sprite;
use crate::objects::unit::Unit;
use crate::orders::action_state::DONE;
use crate::orders::action_state::FAILED;
use crate::world::step::Step;

impl Attack {
    /// The update of an Attack whose target is gone (its target −1 after [`Attack::lost`]): the
    /// timer runs and, past the target's checks, state 1 (case 0) stops the unit's animation and
    /// returns 1: the list deletes the Attack. A building that cannot attack (`armed` false,
    /// `FUN_14120ea30`) ends instead (`set_state(1)`: the caller's [`Attack::disarmed`]) and
    /// returns 1, true. The other states without a target are not modelled.
    pub fn update_targetless(
        &mut self,
        dt: f32,
        armed: bool,
        body: &mut Body,
    ) -> Result<(u8, bool), GenieError> {
        self.timer += dt;
        if !armed {
            self.state = DONE;
            return Ok((1, true));
        }
        if self.state != DONE {
            return Err(GenieError::unsupported(
                "an Attack without a target in a state but 1",
            ));
        }
        body.set_anim_speed(0.0);
        Ok((1, false))
    }

    /// The update (`0x140a82ae0`) of an Attack in state 5 or 11 whose target is gone (its target
    /// null, which [`Attack::lost`] leaves a mover of stance 0 waiting where it saw it): the
    /// timer runs and the checks of the target are skipped; the sub-action list's update
    /// ([`Attack::update_list`]) walks the `MoveTo` to where it was seen. Code 0 goes on; 1
    /// and 2 (it ended, the list deleting it) end the Attack (`set_state` 1, a unit's own Attack
    /// posting 0x1fb with 600 and 0x202 first) and return 1; 3 fails it
    /// ([`Attack::walk_failed`]); 5 and above do nothing. `master_class` is the unit's master's.
    ///
    /// # Errors
    ///
    /// The walk's and the notifies'.
    pub(crate) fn update_gone(
        &mut self,
        c: &mut Step<'_>,
        unit: &mut Unit,
        me: Me,
        r: &AttackReads,
        master_class: i16,
    ) -> Result<u8, GenieError> {
        self.timer += seconds(c.dt);
        let gives_up = GiveUp {
            waits: !r.computer && master_class == class::VILLAGER,
            stance: r.stance,
        };
        match self.update_list(c, unit, me, r, gives_up)? {
            1 | 2 => {
                self.delete_move_to(unit)?;
                if !self.sub_action {
                    unit.post_own(0x1fb, [600, 0, 0x202])?;
                }
                self.finish(unit, c.w.streams.get(Stream::Sprite))?;
                Ok(1)
            }
            3 => self.walk_failed(c, unit),
            4 => Err(QUIET_CODE),
            _ => Ok(0),
        }
    }

    /// The end of a building's Attack that cannot attack (`0x140a82ae0` at `LAB_140a83711`):
    /// `set_state(1)` shows the standing sprite at speed 1 (its sub-actions none), then a unit's
    /// own Attack (not quiet, `+0x60` 0) posts 0x1fb with 600 and its target (`+0x30`).
    pub(crate) fn disarmed(
        &self,
        sprite: &mut Option<ActiveSprite>,
        body: &mut Body,
        rng: &mut Well512,
    ) -> Result<Option<(i32, [i32; 3])>, GenieError> {
        if self.move_to.is_some() {
            return Err(GenieError::unsupported("a building's Attack with a MoveTo"));
        }
        body.set_anim_speed(1.0);
        let idle = self.arms.idle_sprite.ok_or(GenieError::unsupported(
            "an Attack's unit without a standing sprite",
        ))?;
        new_sprite(sprite, body.move_stats_mut(), idle, rng);
        Ok((!self.sub_action).then_some((0x1fb, [600, self.target, 0])))
    }

    /// One update (slot 6, `0x140a82ae0`) of `unit`'s Attack on the step: the timer runs, the
    /// target is still there and where it was seen; then the state's step. Its notifies go to
    /// the unit AI at once, and what it lets go is let go at once. Returns the action list's
    /// code: 0 under way, 5 done after its target died.
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
        r: &AttackReads,
    ) -> Result<u8, GenieError> {
        self.timer += seconds(c.dt);
        // a dead tree (class 15) up to state 5 keeps a non-villager's attack going: its update
        // goes on, the target not alive (`0x140a8342e`)
        let alive = f.victim.state <= 2;
        let gone = !alive
            && (f.victim.class != class::TREE
                || f.own.class == class::VILLAGER
                || 5 < f.victim.state);
        if self.state != KILLED && gone {
            return self.target_died(c, unit, f, r);
        }
        let t =
            *c.w.spatial
                .get_known(self.target)?
                .ok_or(GenieError::unsupported("an Attack whose target is gone"))?;
        if self.state != KILLED
            && (t.position[0] != self.seen[0] || t.position[1] != self.seen[1])
            && let Some(code) = self.target_moved(c, unit, f, r, &t)?
        {
            return Ok(code);
        }
        if let Some(code) = self.ends_early(c, unit, f, r, alive)? {
            return Ok(code);
        }
        match self.state {
            // case 0: the unit's animation stops; the list deletes the Attack
            DONE => {
                set_walker_anim(&mut unit.body, 0.0);
                Ok(1)
            }
            // case 3 (state 4) walks the MoveTo to the target, cases 4 and 10 (states 5 and
            // 11) to where it was seen
            MOVE | GOTO_SEEN | GOTO_POINT => self.walk(c, unit, me, f, r, (t.position, t.radius)),
            READY => self.ready_step(c, unit, f, r, alive),
            SWING => self.swing(c, unit, f, r),
            KILLED => self.killed_step(c, unit),
            // case 0xc: a failed attack posts 0x1f9 with 600 every update (quiet or not), the
            // unit's animation stops and the update returns 3
            FAILED => {
                unit.post_own(crate::unit_ai::ACTION_FAILED, [600, 0, 0])?;
                set_walker_anim(&mut unit.body, 0.0);
                Ok(3)
            }
            // the states without a case (0, 3, 8 to 10, 12, 14 and above) return 0
            _ => Ok(0),
        }
    }

    /// The update's start on a target that died before the attack ended: the attack lets it go
    /// and ends, returning 5 ([`Attack::lost`] for a unit's own Attack).
    pub(super) fn target_died(
        &mut self,
        c: &mut Step<'_>,
        unit: &mut Unit,
        f: &Fight,
        r: &AttackReads,
    ) -> Result<u8, GenieError> {
        // the target died before the attack ended (the update's start): one of stance 0 that
        // moves would wait (set_state 5), not modelled; a villager's unit AI keeps a dead
        // target of Gaia's (another's FUN_140c5e1e0 forgets, not modelled). The attack lets
        // the target go (slot 0x68 with none: a sub-action lets nothing go; the unit's attack
        // count clears, the caller's) and its point, posts no notify as the hunt's
        // sub-action (`+0x60`), ends (set_state 1) and returns 5
        if !self.sub_action {
            return self.lost(c, unit, f, r);
        }
        if f.own.class != class::VILLAGER {
            return Err(GenieError::unsupported(
                "an Attack of a unit other than a villager whose target died",
            ));
        }
        if f.victim.player != 0 {
            return Err(GenieError::unsupported(
                "a villager's Attack whose player-owned target died (its unit AI forgets it)",
            ));
        }
        self.target = -1;
        self.point = [NONE; 3];
        self.finish(unit, c.w.streams.get(Stream::Sprite))?;
        Ok(5)
    }

    /// The update on a target `t` that moved from where it was seen; Some with the update's code
    /// when that ends it.
    pub(super) fn target_moved(
        &mut self,
        c: &mut Step<'_>,
        unit: &mut Unit,
        f: &Fight,
        r: &AttackReads,
        t: &crate::map::spatial::Placed,
    ) -> Result<Option<u8>, GenieError> {
        // the target moved: where it was seen follows (`+0xa4`..`+0xac`); one the unit's
        // player sees (the target's slot 0x1e0, `FUN_1416635e0` at its tile: 0xf), or only
        // explored (0x80) with its master's fog flag (`+0x605`), keeps the attack going, but
        // an attack in state 5 or 11 follows it again (set_state 4, [`Attack::follow`]). One
        // out of sight keeps an attack in a state but 4 and 5 going; a player's unit without a
        // unit AI, or whose unit AI has an order and an empty queue, waits
        // ([`Attack::wait_at_seen`]; one in state 5 keeps going); any other fails
        // ([`Attack::out_of_sight`])
        self.seen = t.position;
        let seen = sees(&c.w.spatial, f, t.position)?;
        if seen && matches!(self.state, GOTO_SEEN | GOTO_POINT) {
            let ground = unit
                .body
                .mover()
                .map(|m| [m.stats.radius[0], m.stats.radius[1]])
                .ok_or(crate::world::ATTACKER_UNMOVING)?;
            let gap = footprint_gap(unit.body.position(), ground, t.position, t.radius);
            self.follow(unit, c.w.streams.get(Stream::Sprite), f, gap)?;
            return Ok(Some(0));
        }
        if !seen && matches!(self.state, MOVE | GOTO_SEEN) {
            let gives_up = match r.ai_order {
                AiOrder::Absent => false,
                AiOrder::Idle => true,
                AiOrder::Order(_) => !r.queue_empty,
            };
            if f.own.player == 0 || gives_up {
                return self.out_of_sight(c, unit, f, r).map(Some);
            }
            if self.state != GOTO_SEEN {
                self.wait_at_seen(unit, f)?;
                return Ok(Some(0));
            }
        }
        Ok(None)
    }

    /// The update's ends before the state's step (Some with the update's code): an allied
    /// target, a target inside another and a building that cannot attack.
    pub(super) fn ends_early(
        &mut self,
        c: &mut Step<'_>,
        unit: &mut Unit,
        f: &Fight,
        r: &AttackReads,
        alive: bool,
    ) -> Result<Option<u8>, GenieError> {
        // LAB_140a834ab: a live target (state 0 to 2) of a player's unit (+0x718 not 0) that
        // its player counts as itself or an ally (FUN_141663dc0 with 1) ends the attack
        // (set_state 1; its own Attack, not quiet, posts 0x1fb with 600 and the target) and
        // returns 1, unless the target is livestock (58) or a farm (49) that was its player's when
        // the Attack took it (+0x9f)
        let spared = matches!(f.victim.class, class::LIVESTOCK | class::FARM) && self.same_owner;
        let rng = c.w.streams.get(Stream::Sprite);
        if alive && f.own.player != 0 && !spared && f.victim.allied[0] {
            self.finish(unit, rng)?;
            if !self.sub_action {
                unit.post_own(0x1fb, [600, self.target, 0])?;
            }
            return Ok(Some(1));
        }
        // a target inside another (+0x2f0): the attack ends (set_state 1) and returns 4, which
        // keeps it in the list; a unit's own Attack (not quiet, +0x60) of a unit whose unit AI's
        // order queue is empty posts 0x1fb with 600 and the target
        if f.victim.inside {
            self.finish(unit, rng)?;
            if !self.sub_action && r.queue_empty {
                unit.post_own(0x1fb, [600, self.target, 0])?;
            }
            return Ok(Some(4));
        }
        // a building that cannot attack (FUN_14120ea30) ends it (set_state 1), its own Attack
        // posting 0x1fb with 600 and the target, and returns 1: the list deletes it
        if !r.armed {
            self.finish(unit, rng)?;
            if !self.sub_action {
                unit.post_own(0x1fb, [600, self.target, 0])?;
            }
            return Ok(Some(1));
        }
        Ok(None)
    }

    /// State 6's step (case 5): readyToAttack ready swings ([`Attack::set_swing`]), waiting
    /// waits, away follows the target ([`Attack::follow`]); ready on a target no longer alive (a
    /// dead tree) ends the attack with code 5.
    pub(super) fn ready_step(
        &mut self,
        c: &mut Step<'_>,
        unit: &mut Unit,
        f: &Fight,
        r: &AttackReads,
        alive: bool,
    ) -> Result<u8, GenieError> {
        let mut g = c.w.attack_ground();
        match self.ready_to_attack(unit, &mut g, f, r, false)? {
            // a target no longer alive (a dead tree): set_state(1), code 5
            Readiness::Ready if !alive => {
                self.finish(unit, g.sprite_rng)?;
                return Ok(5);
            }
            Readiness::Ready => self.set_swing(unit, &mut g, f, r)?,
            Readiness::Waiting => {}
            Readiness::Away(gap) => self.follow(unit, g.sprite_rng, f, gap)?,
        }
        Ok(0)
    }

    /// State 2's step (case 1, the target killed): once the swing's sprite looped the attack lets
    /// its target go and ends.
    pub(super) fn killed_step(
        &mut self,
        c: &mut Step<'_>,
        unit: &mut Unit,
    ) -> Result<u8, GenieError> {
        // case 1: once the swing's sprite looped (or there is none, or it does not
        // animate) the attack lets its target go and ends; the hunt reads code 5
        let done = unit
            .sprite
            .is_none_or(|sp| sp.looped || sp.graphic.flags & 1 == 0);
        if !done {
            return Ok(0);
        }
        // the target goes (slot 0x68 with none: a sub-action lets nothing go; the unit's
        // attack count clears, the caller's) and the attack ends (set_state 1); the Hunt's
        // Attack, a sub-action, then posts 0x1fb with 600 and 0x202 too and returns 5
        if self.sub_action {
            self.target = -1;
            self.finish(unit, c.w.streams.get(Stream::Sprite))?;
            unit.post_own(0x1fb, [600, 0, 0x202])?;
            return Ok(5);
        }
        // a unit's own Attack (not quiet, `+0x60` 0): the target goes (slot 0x68 with
        // none), it posts 0x1fb with 600 and 0x202, ends (set_state 1) and returns 2,
        // which keeps it in the list
        let target = std::mem::replace(&mut self.target, -1);
        let_go(c, unit, target)?;
        unit.post_own(0x1fb, [600, 0, 0x202])?;
        self.finish(unit, c.w.streams.get(Stream::Sprite))?;
        Ok(2)
    }

    /// `set_state(5)` (`0x140a820c0` case 5): the sub-actions go, the range is slot 0x238's
    /// again, and a `MoveTo` to where the target was seen within it starts
    /// ([`Attack::goto_point`]).
    pub(super) fn wait_at_seen(&mut self, unit: &mut Unit, f: &Fight) -> Result<(), GenieError> {
        self.enter(GOTO_SEEN, unit)?;
        self.range = f.own.source.weapon_range;
        self.goto_point(unit)
    }

    /// The `MoveTo` to a point of states 5 and 11 ([`Attack::walk_to_seen`]) of `unit`: its start
    /// finds no target object to be near (`FUN_14118f680` answers `FLT_MAX`), so a unit with a
    /// top speed (`UCOMISS`: not 0) asks for a path (state 3), any other fails (state 13).
    ///
    /// # Errors
    ///
    /// A unit that does not move.
    pub(super) fn goto_point(&mut self, unit: &Unit) -> Result<(), GenieError> {
        self.walk_to_seen(Walker::of(unit)?);
        Ok(())
    }

    /// States 5 and 11 once their `MoveTo` arrived (cases 4 and 10, its update 1 or 2): a target
    /// the unit's player sees (the target's slot 0x1e0: 0xf, or 0x80 with its master's fog flag)
    /// makes it ready (`set_state` 6). Out of sight, a human player's unit in state 5 with a range
    /// of at least [`SEEN_RANGE`] (`COMISS`: a NaN is not) walks onto the point (`set_state` 11:
    /// the sub-actions go, the range is slot 0x238's, a `MoveTo` without a range to where the
    /// target was seen); any other lets the target go (slot 0x68 with none) and fails
    /// (`set_state` 13), a unit's own Attack posting 0x1f9 with 600, and returns 3.
    pub(super) fn arrived_at_seen(
        &mut self,
        c: &mut Step<'_>,
        unit: &mut Unit,
        f: &Fight,
        r: &AttackReads,
        target: [f32; 3],
    ) -> Result<u8, GenieError> {
        if sees(&c.w.spatial, f, target)? {
            self.ready(unit, &mut c.w.attack_ground(), f, r)?;
            return Ok(0);
        }
        if self.state == GOTO_SEEN && f.own.player != 0 && SEEN_RANGE <= self.range && !r.computer {
            self.enter(GOTO_POINT, unit)?;
            self.range = f.own.source.weapon_range;
            self.goto_point(unit)?;
            return Ok(0);
        }
        let target = std::mem::replace(&mut self.target, -1);
        if !self.sub_action {
            let_go(c, unit, target)?;
        }
        self.finish_in(FAILED, unit, c.w.streams.get(Stream::Sprite))?;
        if !self.sub_action {
            unit.post_own(crate::unit_ai::ACTION_FAILED, [600, 0, 0])?;
        }
        Ok(3)
    }
}

/// The target's tile at `at` seen by the unit's player (`f.own`), or only explored with its
/// master's fog flag (the target's slot 0x1e0: 0xf, or 0x80 with `+0x605`).
///
/// # Errors
///
/// A tile the map's sight does not hold.
pub(super) fn sees(
    map: &crate::map::spatial::Spatial,
    f: &Fight,
    at: [f32; 3],
) -> Result<bool, GenieError> {
    let (tx, ty) = (at[0] as i32, at[1] as i32);
    Ok(map.sees_tile(f.own.player, tx, ty)
        || (f.victim.fog_visibility != 0 && map.explored(f.own.player, tx, ty)?))
}

/// A unit's own Attack lets its target go (slot 0x68 with none: its slot 0x180) at once
/// ([`crate::world::lets::release`]); none (−1) lets nothing go.
///
/// # Errors
///
/// The release's.
pub(super) fn let_go(c: &mut Step<'_>, unit: &mut Unit, target: i32) -> Result<(), GenieError> {
    if target == -1 {
        return Ok(());
    }
    let by = unit.releaser(c);
    crate::world::lets::release(c, unit, target, by)
}

/// The squared reach of an attack of `range` (`FUN_140a82ae0` at `0x140a83950`): the range squared,
/// or [`TOUCH`] for a range of at most [`NO_RANGE`].
pub(super) fn squared_reach(range: f32) -> f32 {
    if NO_RANGE < range {
        range * range
    } else {
        TOUCH
    }
}
