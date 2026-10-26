//! readyToAttack (`FUN_140a7f9d0`): the ready state, the follow, the range and sight tests
//! and the facing at its end.

use crate::GenieError;
use crate::combat::attack::{
    Attack, FAILED, HALF, MASTER_1704, MASTER_1706, MOVE, NO_RANGE, NONE, READY, Readiness,
    SHORT_RANGE_GAP, Strike, face_target,
};
use crate::movement::move_to::Walk;
use crate::movement::{Mover, footprint_gap};

impl Attack {
    /// `set_state(6)` (case 6): with a target or a point (`+0x38` not −1, `UCOMISS`: a NaN one
    /// is), the range test ([`Attack::ready_to_attack`]); in reach and ready, the swing
    /// (`set_state` 7). Not ready, a unit whose slot 0x5d8 answers 1 (a master whose move
    /// algorithm has bit 0) clears its `+0x958`, the count that picks the second attack sprite,
    /// which the slice does not hold ([`Attack::set_swing`]); readyToAttack never answers 2, so
    /// the `set_state(13)` after it is not reached. Without either a unit's own Attack posts
    /// 0x1fa with 600 (the action completed) and ends (`set_state` 1, [`Attack::finish`]).
    pub(super) fn ready(
        &mut self,
        walk: &mut Walk<'_>,
        s: &mut Strike<'_>,
    ) -> Result<(), GenieError> {
        self.enter(READY, walk, s)?;
        if self.target == -1 && self.point[0] == NONE {
            if !self.sub_action {
                s.posted
                    .push((crate::unit_ai::ACTION_COMPLETED, [600, 0, 0]));
            }
            return self.finish(walk, s);
        }
        match self.ready_to_attack(&mut walk.reborrow(), s, false)? {
            Readiness::Ready => self.set_swing(walk, s),
            Readiness::Waiting => Ok(()),
            Readiness::Away(gap) => self.follow(walk, s, gap),
        }
    }

    /// readyToAttack's answer to a target out of reach of a unit that may follow it (its notify
    /// 0x1fd posted): `set_state(4)`, the `MoveTo` after it with the range of slot 0x238 (the
    /// master's weapon range).
    pub(super) fn follow(
        &mut self,
        walk: &mut Walk<'_>,
        s: &mut Strike<'_>,
        gap: f32,
    ) -> Result<(), GenieError> {
        self.enter(MOVE, walk, s)?;
        if self.moving(s.own.source.weapon_range, gap) {
            self.arrived_sprite(s)?;
        }
        Ok(())
    }

    /// readyToAttack (`FUN_140a7f9d0`) on an object; `skip_ready` is its third argument (1 from
    /// `set_state(7)`'s range clamp: no ready-at-once test). A target of a player (not Gaia) out
    /// of the unit's player's sight (its slot 0x1e0 neither 0xf nor 0x80 with its master's fog
    /// flag) fails the attack (`set_state` 13) unless the unit AI's stance (`+0x1a9`) is 0, the
    /// top speed above 0 (`COMISS`/`JBE`) and canMoveToAttack 1, which waits
    /// ([`Attack::wait_at_seen`]: `set_state` 5, a `MoveTo` to where it was seen). A unit that moves (a master of type 30 and
    /// up with a speed, `FUN_140fdd080`; the slice reads the unit's top speed, above 0 just when
    /// the master's is) whose target stands on the attack's point (`FUN_140a7bbf0`) or within
    /// half a tile of it on each axis (`FUN_140a7bd10`: a NaN difference is not within) is ready
    /// at once; so is a unit that does not move with such a target in its range ([`in_range`]),
    /// or with a volley under way (`FUN_1412161a0`). Else the reach decides, by the target's gap
    /// (`FUN_14118f680`): below the least range (`COMISS`/`JA`) a unit's own Attack posts 0x1fe
    /// (−1, −1, −1); a unit with a top speed (`UCOMISS`: not 0) whose unit AI may move to attack
    /// (slot 0x1d0) waits, any other fails (`set_state` 13). Beyond the reach (the range, half a
    /// tile more without one) a unit's own Attack posts 0x1fd; stance 1's leash fails a target
    /// too far from the unit AI's `state_position` ([`Attack::off_leash`]); a unit that may move follows the target
    /// (`set_state` 4, [`Readiness::Away`]), any other fails. Within it the target's place becomes
    /// the point and the unit faces it ([`Attack::face`]). A gate's reach (class 39) is to the
    /// nearest of it and its first two parts (its slot 0x680 through `FUN_1415417a0`, `MINSS`:
    /// [`crate::movement::gate_gap`]).
    pub(super) fn ready_to_attack(
        &mut self,
        walk: &mut Walk<'_>,
        s: &mut Strike<'_>,
        skip_ready: bool,
    ) -> Result<Readiness, GenieError> {
        let t = *walk
            .world
            .get_known(self.target)?
            .ok_or(GenieError::unsupported("an Attack whose target is gone"))?;
        let speed = walk
            .mover
            .as_deref()
            .ok_or(crate::turn::ATTACKER_UNMOVING)?
            .stats
            .speed;
        if target_unseen(walk.world, s, &t)? {
            if s.stance == 0 && 0.0 < speed && s.can_move {
                self.wait_at_seen(walk, s)?;
                return Ok(Readiness::Waiting);
            }
            return self.fail(walk, s);
        }
        let at = walk
            .world
            .get(walk.me.id)
            .map_or(walk.me.position, |p| p.position);
        let mover = walk
            .mover
            .as_deref_mut()
            .ok_or(crate::turn::ATTACKER_UNMOVING)?;
        let ground = [mover.stats.radius[0], mover.stats.radius[1]];
        let gap = footprint_gap(at, ground, t.position, t.radius);
        let near = if self.range <= NO_RANGE {
            gap + SHORT_RANGE_GAP
        } else {
            gap
        };
        let mobile = 30 <= s.own.kind && 0.0 < speed;
        if !skip_ready && self.ready_at_once(t.position, near, mobile || s.volley_under_way) {
            return self.face(mover, at, t.position, s);
        }
        if gap < self.min_range {
            if !self.sub_action {
                s.posted.push((0x1fe, [-1; 3]));
            }
            if speed != 0.0 && s.can_move {
                return Ok(Readiness::Waiting);
            }
            return self.fail(walk, s);
        }
        // a gate's reach is to the nearest of it and its first two parts (slot 0x680, MINSS)
        let gap =
            crate::movement::gate_gap(walk.world, (at, ground), (self.target, &t), gap, |g| g);
        if self.reach() < gap {
            if !self.sub_action {
                s.posted.push((0x1fd, [0; 3]));
            }
            if (s.stance == 1 && self.off_leash(t.position, s.state_position))
                || speed == 0.0
                || !s.can_move
            {
                return self.fail(walk, s);
            }
            return Ok(Readiness::Away(gap));
        }
        self.point = t.position;
        self.face(mover, at, t.position, s)
    }

    /// readyToAttack's ready-at-once test: the target stands on the attack's point
    /// (`FUN_140a7bbf0`), or within half a tile of it on each axis (`FUN_140a7bd10`: a NaN
    /// difference is not within) and either `moves_or_volleys` (a unit that moves, or a volley
    /// under way, `FUN_1412161a0`) or the target's gap `near` is in range ([`in_range`]).
    pub(super) fn ready_at_once(&self, to: [f32; 3], near: f32, moves_or_volleys: bool) -> bool {
        let moved = to != self.point;
        let far = (0..3)
            .any(|i| crate::math::fmath::below_or_unordered(HALF, (to[i] - self.point[i]).abs()));
        !moved || (!far && (moves_or_volleys || in_range(near, self.range, self.min_range)))
    }

    /// readyToAttack's `set_state(13)` (as [`Attack::finish_in`]); it answers 0.
    pub(super) fn fail(
        &mut self,
        walk: &mut Walk<'_>,
        s: &mut Strike<'_>,
    ) -> Result<Readiness, GenieError> {
        self.finish_in(FAILED, walk, s)?;
        Ok(Readiness::Waiting)
    }

    /// readyToAttack's ready end (`0x140a801f2`): the unit at `at` faces its target at `to`
    /// (`FUN_141162db0`, [`face_target`]; masters 1704 and 1706 skip it); one not facing it yet
    /// waits; one facing it swings once its attack timer ran out (slot 0x100, `FUN_141082d40`:
    /// `COMISS` with 0 then `SETBE`, so a NaN timer has run out).
    pub(super) fn face(
        &self,
        mover: &mut Mover,
        at: [f32; 3],
        to: [f32; 3],
        s: &Strike<'_>,
    ) -> Result<Readiness, GenieError> {
        if !matches!(s.own.master_id, MASTER_1704 | MASTER_1706)
            && !face_target(mover, at, to, s.world_time)?
        {
            return Ok(Readiness::Waiting);
        }
        Ok(
            if crate::math::fmath::at_most_or_unordered(*s.attack_timer, 0.0) {
                Readiness::Ready
            } else {
                Readiness::Waiting
            },
        )
    }
}

/// readyToAttack's ready-at-once range test of a unit that does not move (`0x140a7fd92`): the
/// gap `near` (half a tile less for a range up to 0.1) is not beyond the range (`COMISS`/`JA`), and
/// the least range is at most 0.1 or at most the gap (`COMISS`/`JBE` each: NaN passes).
pub(super) fn in_range(near: f32, range: f32, min_range: f32) -> bool {
    use crate::math::fmath::at_most_or_unordered;
    at_most_or_unordered(near, range)
        && (at_most_or_unordered(min_range, NO_RANGE) || at_most_or_unordered(min_range, near))
}

/// readyToAttack's sight test: a target of a player (not Gaia) whose tile the unit's player does
/// not see (its slot 0x1e0 neither 0xf) nor has explored while the target's master shows in the
/// fog (0x80 with its fog flag).
///
/// # Errors
///
/// A tile the map's sight does not hold.
pub(super) fn target_unseen(
    world: &crate::spatial::Spatial,
    s: &Strike<'_>,
    t: &crate::spatial::Placed,
) -> Result<bool, GenieError> {
    let (tx, ty) = (t.position[0] as i32, t.position[1] as i32);
    Ok(0 < t.player
        && !world.sees_tile(s.own.player, tx, ty)
        && !(s.victim.fog_visibility != 0 && world.explored(s.own.player, tx, ty)?))
}
