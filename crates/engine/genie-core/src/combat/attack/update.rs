//! One update (slot 6, `0x140a82ae0`): a target gone, a disarmed building, the walks to where
//! the target was seen.

use crate::GenieError;
use crate::combat::attack::{
    Attack, DONE, FAILED, FARM, GOTO_POINT, GOTO_SEEN, KILLED, LIVESTOCK, MOVE, NO_RANGE, NONE,
    READY, Readiness, SEEN_RANGE, SWING, Strike, TOUCH, TREE, VILLAGER, VILLAGER_CLASS, set_sprite,
};
use crate::math::rng::Well512;
use crate::movement::footprint_gap;
use crate::movement::move_to::{Goal, MoveTo, Walk};
use crate::objects::sprite::ActiveSprite;

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
        anim_speed: &mut Option<f32>,
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
        *anim_speed = Some(0.0);
        Ok((1, false))
    }

    /// The end of a building's Attack that cannot attack (`0x140a82ae0` at `LAB_140a83711`):
    /// `set_state(1)` shows the standing sprite at speed 1 (its sub-actions none), then a unit's
    /// own Attack (not quiet, `+0x60` 0) posts 0x1fb with 600 and its target (`+0x30`).
    pub(crate) fn disarmed(
        &self,
        sprite: &mut Option<ActiveSprite>,
        anim_speed: &mut Option<f32>,
        rng: &mut Well512,
    ) -> Result<Option<(i32, [i32; 3])>, GenieError> {
        if self.move_to.is_some() {
            return Err(GenieError::unsupported("a building's Attack with a MoveTo"));
        }
        *anim_speed = Some(1.0);
        let idle = self.arms.idle_sprite.ok_or(GenieError::unsupported(
            "an Attack's unit without a standing sprite",
        ))?;
        set_sprite(sprite, idle, rng);
        Ok((!self.sub_action).then_some((0x1fb, [600, self.target, 0])))
    }

    /// One update (slot 6, `0x140a82ae0`): the timer runs, the target is still there and where it
    /// was seen; then the state's step. Returns the action list's code: 0 under way, 5 done after
    /// its target died.
    pub fn update(&mut self, mut walk: Walk<'_>, s: &mut Strike<'_>) -> Result<u8, GenieError> {
        self.timer += walk.dt;
        // a dead tree (class 15) up to state 5 keeps a non-villager's attack going: its update
        // goes on, the target not alive (`0x140a8342e`)
        let alive = s.victim.state <= 2;
        let gone =
            !alive && (s.victim.class != TREE || s.own.class == VILLAGER || 5 < s.victim.state);
        if self.state != KILLED && gone {
            return self.target_died(&mut walk, s);
        }
        let t = *walk
            .world
            .get_known(self.target)?
            .ok_or(GenieError::unsupported("an Attack whose target is gone"))?;
        if self.state != KILLED
            && (t.position[0] != self.seen[0] || t.position[1] != self.seen[1])
            && let Some(code) = self.target_moved(&mut walk, s, &t)?
        {
            return Ok(code);
        }
        if let Some(code) = self.ends_early(&mut walk, s, alive)? {
            return Ok(code);
        }
        match self.state {
            // case 0: the unit's animation stops; the list deletes the Attack
            DONE => {
                *s.anim_speed = Some(0.0);
                Ok(1)
            }
            // case 3 (state 4) walks the MoveTo to the target, cases 4 and 10 (states 5 and
            // 11) to where it was seen
            MOVE | GOTO_SEEN | GOTO_POINT => self.walk(walk, s, t.position, t.radius),
            READY => self.ready_step(&mut walk, s, alive),
            SWING => self.swing(&mut walk, s),
            KILLED => self.killed_step(&mut walk, s),
            // case 0xc: a failed attack posts 0x1f9 with 600 every update (quiet or not), the
            // unit's animation stops and the update returns 3
            FAILED => {
                s.posted.push((crate::unit_ai::ACTION_FAILED, [600, 0, 0]));
                *s.anim_speed = Some(0.0);
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
        walk: &mut Walk<'_>,
        s: &mut Strike<'_>,
    ) -> Result<u8, GenieError> {
        // the target died before the attack ended (the update's start): one of stance 0 that
        // moves would wait (set_state 5), not modelled; a villager's unit AI keeps a dead
        // target of Gaia's (another's FUN_140c5e1e0 forgets, not modelled). The attack lets
        // the target go (slot 0x68 with none: a sub-action lets nothing go; the unit's attack
        // count clears, the caller's) and its point, posts no notify as the hunt's
        // sub-action (`+0x60`), ends (set_state 1) and returns 5
        if !self.sub_action {
            return self.lost(walk, s);
        }
        if s.own.class != VILLAGER {
            return Err(GenieError::unsupported(
                "an Attack of a unit other than a villager whose target died",
            ));
        }
        if s.victim.player != 0 {
            return Err(GenieError::unsupported(
                "a villager's Attack whose player-owned target died (its unit AI forgets it)",
            ));
        }
        self.target = -1;
        self.point = [NONE; 3];
        self.finish(walk, s)?;
        Ok(5)
    }

    /// The update on a target `t` that moved from where it was seen; Some with the update's code
    /// when that ends it.
    pub(super) fn target_moved(
        &mut self,
        walk: &mut Walk<'_>,
        s: &mut Strike<'_>,
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
        let (tx, ty) = (t.position[0] as i32, t.position[1] as i32);
        let seen = walk.world.sees_tile(s.own.player, tx, ty)
            || (s.victim.fog_visibility != 0 && walk.world.explored(s.own.player, tx, ty)?);
        if seen && matches!(self.state, GOTO_SEEN | GOTO_POINT) {
            let at = walk
                .world
                .get(walk.me.id)
                .map_or(walk.me.position, |p| p.position);
            let ground = walk
                .mover
                .as_ref()
                .map(|m| [m.stats.radius[0], m.stats.radius[1]])
                .ok_or(crate::world::ATTACKER_UNMOVING)?;
            let gap = footprint_gap(at, ground, t.position, t.radius);
            self.follow(walk, s, gap)?;
            return Ok(Some(0));
        }
        if !seen && matches!(self.state, MOVE | GOTO_SEEN) {
            if s.own.player == 0 || s.ai_order.is_some_and(|o| o == -1 || !s.queue_empty) {
                return self.out_of_sight(walk, s).map(Some);
            }
            if self.state != GOTO_SEEN {
                self.wait_at_seen(walk, s)?;
                return Ok(Some(0));
            }
        }
        Ok(None)
    }

    /// The update's ends before the state's step (Some with the update's code): an allied
    /// target, a target inside another and a building that cannot attack.
    pub(super) fn ends_early(
        &mut self,
        walk: &mut Walk<'_>,
        s: &mut Strike<'_>,
        alive: bool,
    ) -> Result<Option<u8>, GenieError> {
        // LAB_140a834ab: a live target (state 0 to 2) of a player's unit (+0x718 not 0) that
        // its player counts as itself or an ally (FUN_141663dc0 with 1) ends the attack
        // (set_state 1; its own Attack, not quiet, posts 0x1fb with 600 and the target) and
        // returns 1, unless the target is livestock (58) or a farm (49) that was its player's when
        // the Attack took it (+0x9f)
        let spared = matches!(s.victim.class, LIVESTOCK | FARM) && self.same_owner;
        if alive && s.own.player != 0 && !spared && s.victim.allied[0] {
            self.finish(walk, s)?;
            if !self.sub_action {
                s.posted.push((0x1fb, [600, self.target, 0]));
            }
            return Ok(Some(1));
        }
        // a target inside another (+0x2f0): the attack ends (set_state 1) and returns 4, which
        // keeps it in the list; a unit's own Attack (not quiet, +0x60) of a unit whose unit AI's
        // order queue is empty posts 0x1fb with 600 and the target
        if s.victim.inside {
            self.finish(walk, s)?;
            if !self.sub_action && s.queue_empty {
                s.posted.push((0x1fb, [600, self.target, 0]));
            }
            return Ok(Some(4));
        }
        // a building that cannot attack (FUN_14120ea30) ends it (set_state 1), its own Attack
        // posting 0x1fb with 600 and the target, and returns 1: the list deletes it
        if !s.armed {
            self.finish(walk, s)?;
            if !self.sub_action {
                s.posted.push((0x1fb, [600, self.target, 0]));
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
        walk: &mut Walk<'_>,
        s: &mut Strike<'_>,
        alive: bool,
    ) -> Result<u8, GenieError> {
        match self.ready_to_attack(&mut walk.reborrow(), s, false)? {
            // a target no longer alive (a dead tree): set_state(1), code 5
            Readiness::Ready if !alive => {
                self.finish(walk, s)?;
                return Ok(5);
            }
            Readiness::Ready => self.set_swing(walk, s)?,
            Readiness::Waiting => {}
            Readiness::Away(gap) => self.follow(walk, s, gap)?,
        }
        Ok(0)
    }

    /// State 2's step (case 1, the target killed): once the swing's sprite looped the attack lets
    /// its target go and ends.
    pub(super) fn killed_step(
        &mut self,
        walk: &mut Walk<'_>,
        s: &mut Strike<'_>,
    ) -> Result<u8, GenieError> {
        // case 1: once the swing's sprite looped (or there is none, or it does not
        // animate) the attack lets its target go and ends; the hunt reads code 5
        let done = s
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
            self.finish(walk, s)?;
            s.posted.push((0x1fb, [600, 0, 0x202]));
            return Ok(5);
        }
        // a unit's own Attack (not quiet, `+0x60` 0): the target goes (slot 0x68 with
        // none), it posts 0x1fb with 600 and 0x202, ends (set_state 1) and returns 2,
        // which keeps it in the list
        s.let_go = Some(self.target);
        self.target = -1;
        s.posted.push((0x1fb, [600, 0, 0x202]));
        self.finish(walk, s)?;
        Ok(2)
    }

    /// Where the `MoveTo` of the state goes and within what: state 4's to the target within the
    /// range, state 5's to [`Attack::goto`] within the range, state 11's onto it.
    pub(super) const fn goal(&self) -> (Goal, f32) {
        match self.state {
            MOVE => (Goal::Target(self.target), self.range),
            GOTO_POINT => (Goal::Point(self.goto), 0.0),
            _ => (Goal::Point(self.goto), self.range),
        }
    }

    /// `set_state(5)` (`0x140a820c0` case 5): the sub-actions go, the range is slot 0x238's
    /// again, and a `MoveTo` to where the target was seen within it starts
    /// ([`Attack::goto_point`]).
    pub(super) fn wait_at_seen(
        &mut self,
        walk: &mut Walk<'_>,
        s: &mut Strike<'_>,
    ) -> Result<(), GenieError> {
        self.enter(GOTO_SEEN, walk, s)?;
        self.range = s.own.source.weapon_range;
        self.goto_point(walk)
    }

    /// The `MoveTo` to a point of states 5 and 11 (`FUN_140aa0d30` to `+0xa4`..`+0xac`, quiet:
    /// `FUN_140a7a170` with 1): the timer is 0, and its start (`FUN_140aa2100` with 1) finds no
    /// target object to be near (`FUN_14118f680` answers `FLT_MAX`), so a unit with a top speed
    /// (`UCOMISS`: not 0) asks for a path (state 3), any other fails (state 13).
    pub(super) fn goto_point(&mut self, walk: &Walk<'_>) -> Result<(), GenieError> {
        self.goto = self.seen;
        self.timer = 0.0;
        let speed = walk
            .mover
            .as_ref()
            .ok_or(crate::world::ATTACKER_UNMOVING)?
            .stats
            .speed;
        self.move_to = Some(if speed == 0.0 {
            MoveTo::Failed
        } else {
            MoveTo::Start
        });
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
        walk: &mut Walk<'_>,
        s: &mut Strike<'_>,
        target: [f32; 3],
    ) -> Result<u8, GenieError> {
        let (tx, ty) = (target[0] as i32, target[1] as i32);
        let seen = walk.world.sees_tile(s.own.player, tx, ty)
            || (s.victim.fog_visibility != 0 && walk.world.explored(s.own.player, tx, ty)?);
        if seen {
            self.ready(walk, s)?;
            return Ok(0);
        }
        if self.state == GOTO_SEEN && s.own.player != 0 && SEEN_RANGE <= self.range && !s.computer {
            self.enter(GOTO_POINT, walk, s)?;
            self.range = s.own.source.weapon_range;
            self.goto_point(walk)?;
            return Ok(0);
        }
        s.let_go = Some(self.target);
        self.target = -1;
        self.finish_in(FAILED, walk, s)?;
        if !self.sub_action {
            s.posted.push((crate::unit_ai::ACTION_FAILED, [600, 0, 0]));
        }
        Ok(3)
    }

    /// The `MoveTo`'s answer to the motion step giving up ([`MoveWait::give_up`]).
    ///
    /// [`MoveWait::give_up`]: crate::movement::move_to::MoveWait::give_up
    pub(super) const fn give_up(&mut self, s: &Strike<'_>) {
        let waits = !s.computer && s.own.class == VILLAGER_CLASS;
        self.move_to = Some(self.move_wait.give_up(waits, s.stance));
    }
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
