//! The Attack's constructor (`FUN_140a7c210`) and its start (slot 5): its sprites and frames
//! and what it reads of its unit and its target.

use crate::GenieError;
use crate::combat::attack::{
    Arms, Attack, Begun, Commanded, Engagement, FAILED, GATE, GOTO_SEEN, LEASH_SLACK, MASTER_1704,
    MASTER_1706, NONE, Strike, face_target, set_sprite,
};
use crate::math::rng::Well512;
use crate::move_to::{MoveTo, MoveWait, Walk};
use crate::movement::{Mover, footprint_gap, footprint_gap_squared};
use crate::sprite::{ActiveSprite, Graphic};

impl Attack {
    /// The constructor (`FUN_140a7c210`) on `target`: the range and least range of `e`, its
    /// target's place as where it was seen (`+0xa4`..`+0xac`), the point −1 on each axis, the
    /// sprites and frames of `arms`, whether the target is its unit's player's (`+0x9f`); a
    /// unit's own Attack (`+0x60` 0), its `MoveTo` not repathing at once (`+0x61` 0) and running
    /// (slot 0x7d0's sprite).
    pub(crate) fn own(target: i32, e: &Engagement, arms: Arms) -> Self {
        Self {
            target,
            target2: -1,
            state: 0,
            timer: 0.0,
            range: e.range,
            min_range: e.min_range,
            point: [NONE; 3],
            seen: e.target_position,
            goto: [NONE; 3],
            move_to: None,
            move_wait: MoveWait::default(),
            pending: false,
            swing: 0.0,
            rest: 0.0,
            arms,
            sub_action: false,
            repaths: false,
            runs: true,
            same_owner: e.same_owner,
        }
    }

    /// The constructor (`FUN_140a7c210`, from Hunt's `set_state` 12 with the master's weapon range,
    /// least range, missile and frame: a quiet sub-action, `+0x61` set, its `MoveTo` showing the
    /// hunt's sprite) and the start (slot 5 with 1, `0x140a7f8c0`, [`Attack::begin`]); the target
    /// is the object itself (the area-effect and gate cases are not reached by a villager) and in
    /// sight (a hunt's targets are, and none is a gate, whose parts' gaps the hunt's callers do
    /// not hand on). The unit's `+0x900` (attack count) is cleared and its sprites
    /// set; the slice holds neither. True with it: the last `MoveTo` started arrived, and the
    /// caller shows its idle sprite ([`Attack::arrived_sprite`]).
    ///
    /// # Errors
    ///
    /// What the start does not model, and what no hunt's caller takes: a hunter that cannot move
    /// (its Attack's `set_state` 6 or 13 at the start), and readyToAttack's failure before the
    /// start's `set_state(4)` (the unit's `+0x7b8` at 1, [`Commanded::failed_first`]).
    pub fn start(
        target: i32,
        e: &Engagement,
        arms: Arms,
        mover: &mut Mover,
    ) -> Result<(Self, bool), GenieError> {
        let mut a = Self {
            sub_action: true,
            repaths: true,
            runs: false,
            ..Self::own(target, e, arms)
        };
        if e.target_class == GATE {
            return Err(GenieError::unsupported(
                "a hunt's Attack on a gate (its parts' gaps)",
            ));
        }
        let mut posted = Vec::new();
        let begun = a.begin((e, e.gap), (true, 0), mover, &mut posted)?;
        if begun.ready {
            return Err(GenieError::unsupported(
                "an attack that starts in reach and cannot move (set_state 6)",
            ));
        }
        if begun.failed {
            return Err(GenieError::unsupported(
                "an attack whose unit cannot move to its target",
            ));
        }
        if begun.failed_first {
            return Err(GenieError::unsupported(
                "a hunt's Attack readyToAttack failed before its start's state 4 (+0x7b8 at 1)",
            ));
        }
        Ok((a, begun.arrived))
    }

    /// A unit's own Attack (the attack command, `FUN_141081a80`): the constructor
    /// (`FUN_140a7c210` with sub-action 0: the master's weapon range and least range, its
    /// sprites, missile and frames, its `MoveTo`'s the running one, slot 0x7d0, or the walking
    /// one: the running speed either way; `+0x61` clear) and the start ([`Attack::begin`]) on a
    /// target of `target_owner`, `visible` to the unit's player, in `world` (a gate's reach is to
    /// its nearest part too).
    ///
    /// # Errors
    ///
    /// What the start does not model, and a target the world does not hold.
    pub fn command(
        (target, world): (i32, &crate::spatial::Spatial),
        e: &Engagement,
        arms: Arms,
        (visible, target_owner): (bool, usize),
        mover: &mut Mover,
    ) -> Result<Commanded, GenieError> {
        let t = *world
            .get_known(target)?
            .ok_or(GenieError::unsupported("an Attack whose target is gone"))?;
        let ground = [mover.stats.radius[0], mover.stats.radius[1]];
        let reach_gap =
            crate::movement::gate_gap(world, (e.position, ground), (target, &t), e.gap, |g| g);
        let mut attack = Self::own(target, e, arms);
        let mut posted = Vec::new();
        let begun = attack.begin((e, reach_gap), (visible, target_owner), mover, &mut posted)?;
        Ok(Commanded {
            attack,
            arrived: begun.arrived,
            posted,
            ready: begun.ready,
            failed: begun.failed,
            failed_first: begun.failed_first,
        })
    }

    /// The start (slot 5 with 1, `0x140a7f8c0`) after its unit AI and place test, its `+0x900`
    /// and its standing sprite (the caller's): readyToAttack (`FUN_140a7f9d0`, third argument 0),
    /// then a unit with a top speed (slot 0x208, `UCOMISS`: a NaN one has some) whose unit AI may
    /// move to attack (slot 0x1d0) turns to state 4 (`set_state(4)`: a `MoveTo` to the target, the
    /// old one's teardown stopping the unit); any other goes ready (`set_state(6)`,
    /// [`Begun::ready`]) when readyToAttack reached its ready end (its second argument 1), else
    /// fails (`set_state(13)`, [`Begun::failed`]).
    ///
    /// readyToAttack at the start: a target of a player (`target_owner` above 0) out of the
    /// player's sight (`visible` false: its slot 0x1e0 neither 0xf nor 0x80 with its master's fog
    /// flag) makes a unit of stance 0 with a speed above 0 (`COMISS`/`JBE`: not a NaN one) whose unit AI
    /// may move to attack go where it was seen (`set_state(5)`: a `MoveTo` to that point, which has
    /// no target, so `f32::MAX` from it at its start: state 3), any other fails
    /// ([`Begun::failed_first`]). The constructor's point (−1 on each axis) is never the target's
    /// place, nor within half a tile of it, so no ready-at-once test passes and the reach
    /// decides, by the gap (`FUN_14118f680`): below the least range (`COMISS`/`JA`) a unit's own
    /// Attack posts 0x1fe (−1, −1, −1) and a unit that may not move fails; beyond the reach (the
    /// range, half a tile more up to [`NO_RANGE`]; `COMISS`/`JBE`: a NaN gap is within) it posts
    /// 0x1fd, then stance 1's leash fails a target [`LEASH_SLACK`] past the range from the unit
    /// AI's `state_position` (`FUN_14099ef70`'s root, `COMISS`/`JNC`: a NaN distance holds), a unit
    /// that may move turns to state 4, any other fails. The reach is to `reach_gap`, a gate's
    /// nearest of it and its first two parts ([`crate::movement::gate_gap`]). Within it the
    /// target's place becomes the point and the unit faces it (`FUN_141162db0`, but masters 1704
    /// and 1706): its ready end.
    ///
    /// [`NO_RANGE`]: crate::combat::attack::NO_RANGE
    pub(super) fn begin(
        &mut self,
        (e, reach_gap): (&Engagement, f32),
        (visible, target_owner): (bool, usize),
        mover: &mut Mover,
        posted: &mut Vec<(i32, [i32; 3])>,
    ) -> Result<Begun, GenieError> {
        let movable = e.speed != 0.0 && e.can_move;
        let mut failed_first = false;
        let mut ready_end = false;
        if 0 < target_owner && !visible {
            if e.stance == 0 && 0.0 < e.speed && e.can_move {
                self.state = GOTO_SEEN;
                self.range = e.range;
                self.timer = 0.0;
                self.move_to = Some(MoveTo::Start);
            } else {
                failed_first = true;
            }
        } else if self.min_range > e.gap {
            if !self.sub_action {
                posted.push((0x1fe, [-1; 3]));
            }
            failed_first = !movable;
        } else if !crate::math::fmath::at_most_or_unordered(reach_gap, self.reach()) {
            if !self.sub_action {
                posted.push((0x1fd, [0; 3]));
            }
            if (e.stance == 1 && self.off_leash(e.target_position, e.state_position)) || !movable {
                failed_first = true;
            } else {
                let _ = self.set_move(e, mover)?;
            }
        } else {
            self.point = e.target_position;
            if !matches!(e.master, MASTER_1704 | MASTER_1706) {
                let _ = face_target(mover, e.position, e.target_position, e.world_time)?;
            }
            ready_end = true;
        }
        if movable {
            let arrived = self.set_move(e, mover)?;
            return Ok(Begun {
                arrived,
                ready: false,
                failed: false,
                failed_first,
            });
        }
        Ok(Begun {
            arrived: false,
            ready: ready_end,
            failed: !ready_end,
            failed_first: false,
        })
    }

    /// readyToAttack's leash of stance 1 on a target at `at` out of reach: its distance from the
    /// unit AI's `state_position` (`+0x1ac`, `+0x1b0`; `FUN_14099ef70` of y's square plus x's) at
    /// least the range plus [`LEASH_SLACK`] (`COMISS`/`JNC`: a NaN distance is within).
    pub(super) fn off_leash(&self, at: [f32; 3], centre: [f32; 2]) -> bool {
        let (dx, dy) = (at[0] - centre[0], at[1] - centre[1]);
        crate::math::fmath::sqrt(dy * dy + dx * dx) >= self.range + LEASH_SLACK
    }

    /// The start's `set_state(13)` for a unit that cannot move out of reach of its target
    /// ([`Commanded::failed`]), as [`Attack::finish_in`]: the state, the unit's animation at speed
    /// 1 and its standing sprite (a unit that cannot move has no `MoveTo` to stop). Its next update
    /// posts 0x1f9 and removes it.
    ///
    /// # Errors
    ///
    /// When the unit's master has no standing sprite.
    pub fn fail_at_start(
        &mut self,
        sprite: &mut Option<ActiveSprite>,
        anim_speed: &mut Option<f32>,
        rng: &mut Well512,
    ) -> Result<(), GenieError> {
        self.state = FAILED;
        *anim_speed = Some(1.0);
        let idle = self.arms.idle_sprite.ok_or(GenieError::unsupported(
            "an Attack's unit without a standing sprite",
        ))?;
        set_sprite(sprite, idle, rng);
        Ok(())
    }

    /// The start's `set_state(6)` for a unit that cannot move ([`Commanded::ready`]): the range
    /// test again, and the swing when it is ready.
    ///
    /// # Errors
    ///
    /// What the range test and the swing do not model.
    pub fn enter_ready(
        &mut self,
        mut walk: Walk<'_>,
        s: &mut Strike<'_>,
    ) -> Result<(), GenieError> {
        self.ready(&mut walk, s)
    }
}

/// The Attack constructor's sprites and frames (`FUN_140a7c210`) for a unit that shows its
/// master's sprites ([`arms_looking`] without a terrain look).
#[must_use]
pub fn arms(
    master: &crate::masters::Master,
    graphics: &std::collections::BTreeMap<i16, Graphic>,
    sprites: [i16; 4],
    moving: i16,
) -> Arms {
    arms_looking(master, None, graphics, sprites, moving)
}

/// The Attack constructor's sprites and frames (`FUN_140a7c210`): the `MoveTo`'s sprite `moving`
/// (the hunt's walking sprite from Hunt's `set_state` 12, the task's moving one from the task
/// factory, none from the attack command), else the unit's running sprite (slot 0x7d0), else its
/// walking one (slot 0x7c8; both its terrain look's while it has one, `look`,
/// [`crate::sprite::running`]); the task's proceeding sprite (else the master's attack sprite,
/// `+0x7f8`), the master's second attack sprite and standing sprite, missile, firing and
/// interrupting frames.
#[must_use]
pub fn arms_looking(
    master: &crate::masters::Master,
    look: Option<&crate::combat::abilities::Look>,
    graphics: &std::collections::BTreeMap<i16, Graphic>,
    sprites: [i16; 4],
    moving: i16,
) -> Arms {
    let g = |id: i16| (0 <= id).then(|| graphics.get(&id).copied()).flatten();
    let slot = |id: Option<i16>| id.and_then(g);
    Arms {
        move_sprite: g(moving)
            .or_else(|| slot(crate::sprite::running(master, look)))
            .or_else(|| slot(crate::sprite::walking(master, look, false))),
        sprite: g(sprites[1]).or_else(|| g(master.attack_sprite)),
        second_sprite: g(master.attack_sprite2),
        idle_sprite: g(master.standing_sprite),
        idle_attack_sprite: g(master.idle_attack_sprite),
        missile: master.missile,
        fire_frame: master.fire_frame,
        interrupt_frame: master.interrupt_frame,
    }
}

/// What an Attack of a unit (its unit AI, top speed and master, placed at `me`) reads of it and
/// its target (placed at `them`).
pub fn engagement(
    ai: Option<&crate::unit_ai::UnitAi>,
    speed: f32,
    master: &crate::masters::Master,
    me: &crate::spatial::Placed,
    them: &crate::spatial::Placed,
    world_time: u32,
) -> Result<Engagement, GenieError> {
    if matches!(master.charge_type, 6 | 7) {
        return Err(GenieError::unsupported(
            "an attacker whose charge adds to its range (slot 0x2f8)",
        ));
    }
    let (gap_squared, gap) = gaps(me.position, me.radius, them.position, them.radius);
    Ok(Engagement {
        gap_squared,
        gap,
        position: me.position,
        target_position: them.position,
        speed,
        range: master.weapon_range,
        min_range: master.min_range,
        can_move: ai.is_some_and(|ai| ai.current_order != -1 || ai.stance <= 1),
        stance: ai.map_or(0, |ai| ai.stance),
        state_position: ai.map_or([0.0; 2], |ai| ai.state_position),
        world_time,
        master: master.id,
        target_class: them.class,
        same_owner: me.player == them.player,
    })
}

/// The gap between two footprints, for an [`Engagement`].
#[must_use]
pub fn gaps(a: [f32; 3], a_radius: [f32; 2], b: [f32; 3], b_radius: [f32; 2]) -> (f32, f32) {
    let squared = footprint_gap_squared(a, a_radius, b, b_radius);
    (squared, footprint_gap(a, a_radius, b, b_radius))
}
