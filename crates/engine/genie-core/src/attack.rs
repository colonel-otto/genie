//! The Attack action (`RGE_Action_Attack`, type 9, vtable `0x142ed47a8`) of a melee unit: a
//! hunting villager's sub-action on a tree or on its livestock ([`crate::hunt`]).
//! Built away from its target, it walks there with a `MoveTo` sub-action ([`crate::move_to`]); in
//! reach it swings (state 7): the hit lands on the attack sprite's frame ([`crate::sprite`]), and
//! the unit waits out its reload between swings (state 6). A target that dies ends the swing
//! (state 2) and the action (`set_state` 1, code 5 to the hunt).
//!
//! 158041: constructor `FUN_140a7c210`, start (slot 5) `0x140a7f8c0`, update (slot 6)
//! `0x140a82ae0`, `set_state` (slot 16) `0x140a820c0`; the range test `FUN_140a7f9d0`
//! (`readyToAttack`); the hit (object slot 0x858, `FUN_14120c0a0` -> `FUN_141081020` -> slot 0x860
//! `FUN_1412115e0` -> `FUN_141084230` -> the target's slot 0xe8, whose damage is
//! [`crate::damage`]'s).

use crate::damage::{self, Victim};
use crate::move_to::{Goal, MoveTo, Walk, start_walk, target_moved, walk_on};
use crate::movement::{Mover, footprint_gap, footprint_gap_squared};
use crate::rng::Well512;
use crate::sprite::{ActiveSprite, Graphic};
use crate::unit_ai::Unsupported;

/// The Attack action's type, which word 9 adds.
pub const ATTACK: i16 = 9;

/// A range at most this counts as none (`DAT_142eb5690`): the range test widens the gap by
/// `-0.5` (`DAT_142eb6e58`) and the reach by `0.5` (`DAT_142e7f384`).
const NO_RANGE: f32 = 0.1;
const HALF: f32 = 0.5;
/// The squared reach of an attack without a range (`DAT_142ed48e4`): `0x3c23d70b`, which is
/// `0.1f * 0.1f`, one ULP above `0.01f`.
const TOUCH: f32 = crate::image::attack::TOUCH;
/// A `MoveTo` starts arrived within this gap (`DAT_142eb99cc`).
const ARRIVED: f32 = crate::image::attack::ARRIVED;
/// The gap below which the game takes a distance as zero (`DAT_142eb0c60`).
const EPSILON: f32 = crate::image::attack::EPSILON;
/// The point an attack on an object has none of (`DAT_14330ec20`).
const NONE: f32 = crate::image::attack::NONE;
/// The hit chance's scale (`0x7fffffff`, the low word of a 64-bit Object-stream draw).
const DRAW_SCALE: i64 = 0x7fff_ffff;
/// The states the slice models: 1 done, 2 the target died, 4 on the way, 6 waiting to swing, 7
/// swinging.
pub const DONE: u8 = 1;
/// State 2: the target died.
pub const KILLED: u8 = 2;
/// State 4: on the way.
pub const MOVE: u8 = 4;
/// State 6: waiting to swing.
pub const READY: u8 = 6;
/// State 7: swinging.
pub const SWING: u8 = 7;

/// The sprites and frames the Attack takes from its constructor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Arms {
    /// `+0x70`: the attack sprite (the hunt's task's proceeding sprite); `+0x78`, a second one
    /// the unit picks from; `+0x80`, the sprite it stands with when done; `+0x88`, the sprite it
    /// waits with between swings (the master's idle attack sprite).
    pub sprite: Option<Graphic>,
    /// `+0x78`: a second attack sprite the unit picks from.
    pub second_sprite: Option<Graphic>,
    /// `+0x80`: the sprite it stands with when done.
    pub idle_sprite: Option<Graphic>,
    /// `+0x88`: the sprite it waits with between swings.
    pub idle_attack_sprite: Option<Graphic>,
    /// `+0x98`, the master's missile (`missile_id`, -1 for a melee unit); `+0x9a`, the frame that
    /// fires (`fire_missile_at_frame`); `+0x9c`, the frame that may interrupt
    /// (`interrupt_frame`), -1 for none.
    pub missile: i16,
    /// `+0x98`: the frame it fires at (`fire_missile_at_frame`).
    pub fire_frame: i16,
    /// `+0x9c`: the frame that may interrupt (`interrupt_frame`), −1 for none.
    pub interrupt_frame: i16,
}

/// The Attack action (`RGE_Action_Attack`, type 9) as the hunt's sub-action.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Attack {
    /// The target object (`+0x20`, its id at `+0x30`).
    pub target: i32,
    /// The action state (`+0x18`).
    pub state: u8,
    /// `+0x44`: seconds in the state, the update adds dt.
    pub timer: f32,
    /// `+0x90`: the range it attacks from; `+0x94`, the least.
    pub range: f32,
    /// `+0x94`: the least range it attacks from.
    pub min_range: f32,
    /// `+0x38`..`+0x40`: the point it attacks; -1 for none.
    pub point: [f32; 3],
    /// `+0xa4`..`+0xac`: where it last saw its target.
    pub seen: [f32; 3],
    /// The `MoveTo` sub-action to the target, its range the attack's.
    pub move_to: Option<MoveTo>,
    /// `+0x9e`: a hit is still to land in this swing.
    pub pending: bool,
    /// `+0xb0` and `+0xb4`: the time of a swing past its sprite's length; a melee swing counts
    /// `+0xb0` down until the hit.
    pub swing: f32,
    /// `+0xb4`: the rest after the swing's sprite.
    pub rest: f32,
    /// The sprites and frames from its constructor.
    pub arms: Arms,
}

/// What the Attack reads of its unit and its target when it starts.
#[derive(Clone, Copy, Debug)]
pub struct Engagement {
    /// The gap between the two footprints squared (`FUN_14118f8c0`) and its root (`FUN_14118f680`).
    pub gap_squared: f32,
    /// The gap's root (`FUN_14118f680`).
    pub gap: f32,
    /// The unit's and the target's positions.
    pub position: [f32; 3],
    /// The target's place.
    pub target_position: [f32; 3],
    /// The unit's top speed (slot 0x208), its master's weapon range with any bonus against the
    /// target (slot 0x238) and its least range.
    pub speed: f32,
    /// Its master's weapon range with any bonus against the target (slot 0x238).
    pub range: f32,
    /// Its least range.
    pub min_range: f32,
    /// The unit AI's `canMoveToAttack` (`FUN_140c48420`): it has an order, or no order but a stance
    /// below 2.
    pub can_move: bool,
    /// The unit AI's stance (`+0x1a9`), whose 1 adds a leash test.
    pub stance: u8,
}

/// What the Attack's update reads of its unit besides its movement, and changes.
#[derive(Debug)]
pub struct Strike<'a> {
    /// The unit's active sprite and animation speed (`+0x7b8`), None while the slice does not
    /// know them.
    pub sprite: &'a mut Option<ActiveSprite>,
    /// The unit's animation speed (`+0x7b8`), None while the slice does not know it.
    pub anim_speed: &'a mut Option<f32>,
    /// `+0x91c`, the time until it may attack again (slot 0x100: at most 0); the game's world
    /// time when it may again (`+0x920`, slot 0x248) is not held.
    pub attack_timer: &'a mut f32,
    /// Slot 0x218 (`FUN_141086c90`): the master's reload time (`speed_of_attack`) with the
    /// unit's reload stinger and powerup.
    pub reload: f32,
    /// The master's hit chance (`+0x83e`).
    pub hit_chance: i16,
    /// The target, which the hit changes.
    pub victim: &'a mut Victim,
    /// The objects about the unit a blast of it may reach (its master has an area effect),
    /// besides the unit and its target; empty for others.
    pub blasted: &'a mut Vec<Victim>,
    /// The Sprite stream, which a sprite with a random first frame draws from; the hit draws
    /// from the walk's Object stream.
    pub sprite_rng: &'a mut Well512,
    /// The unit as a combat object: what its hits read of it (its attack, its master's weapons,
    /// its place and its player's elevation attributes) and what a retaliation, reflected damage
    /// or its charge changes.
    pub own: Victim,
    /// The target of a swing this update, whose executed action (notify 0x200 with 600 and the
    /// target) the unit posts to its unit AI.
    pub executed: Option<i32>,
    /// The target a ranged unit's swing fires at this update (slot 0x858, `FUN_141081020`): the
    /// missile is the caller's.
    pub fire: Option<i32>,
    /// The unit AI's canMoveToAttack (slot 0x1d0, `FUN_140c48420`) and stance (`+0x1a9`), which
    /// readyToAttack reads of a target out of reach.
    pub can_move: bool,
    /// The unit AI's stance (`+0x1a9`).
    pub stance: u8,
}

/// The masters whose swings post no executed action (`0x60e5`, `0x62cb` raw).
const QUIET_SWINGS: [i16; 2] = [73, 559];

/// Slot 0x68 (`FUN_141217000`): the unit's sprite becomes `g` unless it is already; a new sprite
/// starts a new instance (`FUN_1413b2ef0`).
pub(crate) fn set_sprite(sprite: &mut Option<ActiveSprite>, g: Graphic, rng: &mut Well512) {
    if sprite.is_some_and(|s| s.graphic.id == g.id) {
        return;
    }
    *sprite = Some(ActiveSprite::start(g, rng));
}

/// What readyToAttack answers: ready to swing, waiting for the attack timer, or the target out of
/// reach at this gap (the unit follows it).
enum Readiness {
    Ready,
    Waiting,
    Away(f32),
}

/// The gate class, whose parts' gaps readyToAttack measures.
const GATE: i16 = 39;
/// The villagers' class, whose attack ends on any dead target.
const VILLAGER: i16 = 4;

impl Attack {
    /// The constructor (`FUN_140a7c210`, from Hunt's `set_state` 12 with the master's weapon range,
    /// least range, missile and frame) and the start (slot 5 with 1, `0x140a7f8c0`): the target is
    /// the object itself (the area-effect and gate cases are not reached by a villager); a range of
    /// 0 becomes slot 0x238's. The start runs the range test: a target in reach becomes the
    /// point and the unit faces it, a unit out of reach that may move turns to state 4; a unit that
    /// may move turns to state 4 again in the start, each time with a new `MoveTo` (whose teardown
    /// stops the unit). The unit's `+0x900` (attack count) is cleared and its sprites set; the
    /// slice holds neither. True with it: the last `MoveTo` started arrived, and the caller shows
    /// its idle sprite ([`Attack::arrived_sprite`]).
    pub fn start(
        target: i32,
        e: &Engagement,
        arms: Arms,
        mover: &mut Mover,
    ) -> Result<(Self, bool), Unsupported> {
        let mut a = Self {
            target,
            state: 0,
            timer: 0.0,
            range: e.range,
            min_range: e.min_range,
            point: [NONE; 3],
            seen: e.target_position,
            move_to: None,
            pending: false,
            swing: 0.0,
            rest: 0.0,
            arms,
        };
        // readyToAttack (`FUN_140a7f9d0`): the target has moved from the attack's point (none,
        // FUN_140a7bbf0 and FUN_140a7bd10), so the reach decides
        if a.min_range > e.gap {
            return Err(Unsupported(
                "an attack on a target closer than the least range",
            ));
        }
        let movable = e.speed != 0.0 && e.can_move;
        if a.reach() >= e.gap {
            // in reach: the target's place becomes the point and the unit faces it
            // (FUN_141162db0)
            a.point = e.target_position;
            face_target(mover, e.position, e.target_position)?;
            if !movable {
                return Err(Unsupported(
                    "an attack that starts in reach and cannot move (set_state 6)",
                ));
            }
        } else {
            if e.stance == 1 {
                return Err(Unsupported("an attack's leash test (stance 1)"));
            }
            if !movable {
                return Err(Unsupported(
                    "an attack whose unit cannot move to its target",
                ));
            }
            let _ = a.set_move(e, mover)?;
        }
        // the start's own test of the speed and the unit AI (slots 0x208 and 0x1d0): state 4
        let arrived = a.set_move(e, mover)?;
        Ok((a, arrived))
    }

    /// A `MoveTo` of the attack that starts arrived (its `set_state` 1, not quiet): the unit shows
    /// its idle sprite (slot 0x68) and its `+0x7b8` clears.
    pub(crate) fn arrived_sprite(&self, s: &mut Strike<'_>) -> Result<(), Unsupported> {
        let g = self
            .arms
            .idle_sprite
            .ok_or(Unsupported("an attacker without its standing sprite"))?;
        set_sprite(s.sprite, g, s.sprite_rng);
        if let Some(a) = s.anim_speed.as_mut() {
            *a = 0.0;
        }
        Ok(())
    }

    /// How far the unit reaches: the range, half a tile more without one.
    fn reach(&self) -> f32 {
        if self.range <= NO_RANGE {
            self.range + HALF
        } else {
            self.range
        }
    }

    /// `set_state(4)` (`0x140a820c0` case 4): the sub-actions go (a `MoveTo`'s teardown stops the unit,
    /// slot 0x5d0), the range is slot 0x238's again, and a `MoveTo` to the target with that range
    /// starts (`FUN_140aa0df0`, start `FUN_140aa2100`): arrived within 0.01, else in state 3 for a
    /// unit with speed. True when it arrived.
    fn set_move(&mut self, e: &Engagement, mover: &mut Mover) -> Result<bool, Unsupported> {
        if self.move_to.take().is_some() {
            mover.stop()?;
        }
        self.state = MOVE;
        Ok(self.moving(e.range, e.gap))
    }

    /// `set_state(4)`'s own part: the range (slot 0x238's), the timer, the `MoveTo` to the target,
    /// which starts arrived (state 1, true) within 0.01 of a `gap` away.
    fn moving(&mut self, range: f32, gap: f32) -> bool {
        self.range = range;
        self.timer = 0.0;
        let arrived = gap <= ARRIVED;
        self.move_to = Some(if arrived {
            MoveTo::Arrived
        } else {
            MoveTo::Start
        });
        arrived
    }

    /// The teardown of the sub-actions (`FUN_140a73fc0` on the action list): a `MoveTo` stops the
    /// unit.
    pub fn tear_down(&mut self, mover: Option<&mut Mover>) -> Result<(), Unsupported> {
        if self.move_to.take().is_some()
            && let Some(m) = mover
        {
            m.stop()?;
        }
        Ok(())
    }

    /// The start of every `set_state` (`0x140a820c0`): the sub-actions go (a `MoveTo`'s teardown stops
    /// the unit and its animation, `+0x7b8`), then the state.
    fn enter(
        &mut self,
        state: u8,
        walk: &mut Walk<'_>,
        s: &mut Strike<'_>,
    ) -> Result<(), Unsupported> {
        if self.move_to.take().is_some() {
            walk.mover
                .as_deref_mut()
                .ok_or(Unsupported("an attacker without movement state"))?
                .stop()?;
            if let Some(a) = s.anim_speed.as_mut() {
                *a = 0.0;
            }
        }
        self.state = state;
        Ok(())
    }

    /// `set_state(1)` (case 1): the unit's animation runs at speed 1 and it takes the sprite it
    /// stands with.
    fn finish(&mut self, walk: &mut Walk<'_>, s: &mut Strike<'_>) -> Result<(), Unsupported> {
        self.enter(DONE, walk, s)?;
        *s.anim_speed = Some(1.0);
        let idle = self
            .arms
            .idle_sprite
            .ok_or(Unsupported("an Attack's unit without a standing sprite"))?;
        set_sprite(s.sprite, idle, s.sprite_rng);
        Ok(())
    }

    /// `set_state(6)` (case 6): the range test; in reach and ready, the swing (`set_state` 7).
    fn ready(&mut self, walk: &mut Walk<'_>, s: &mut Strike<'_>) -> Result<(), Unsupported> {
        self.enter(READY, walk, s)?;
        match self.ready_to_attack(&mut walk.reborrow(), s)? {
            Readiness::Ready => self.set_swing(walk, s),
            Readiness::Waiting => Ok(()),
            Readiness::Away(gap) => self.follow(walk, s, gap),
        }
    }

    /// readyToAttack's answer to a target out of reach: `set_state(4)`, the `MoveTo` after it with
    /// the range of slot 0x238 (the master's weapon range).
    fn follow(
        &mut self,
        walk: &mut Walk<'_>,
        s: &mut Strike<'_>,
        gap: f32,
    ) -> Result<(), Unsupported> {
        self.enter(MOVE, walk, s)?;
        if self.moving(s.own.source.weapon_range, gap) {
            self.arrived_sprite(s)?;
        }
        Ok(())
    }

    /// readyToAttack (`FUN_140a7f9d0`) on an object: a target of a player (not Gaia) out of the
    /// unit's player's sight is not modelled. A unit that moves (a master of type 30 and up with a
    /// speed, `FUN_140fdd080`; the slice reads the unit's top speed) whose target stands on the
    /// attack's point (`FUN_140a7bbf0`) or within half a tile of it on each axis
    /// (`FUN_140a7bd10`) is ready at once; else the reach decides: the target's gap
    /// (`FUN_14118f680`) below the least range is not modelled; beyond the reach (the range, half
    /// a tile more without one) the unit follows it (`set_state` 4, [`Readiness::Away`]) when it may
    /// move (its speed and canMoveToAttack, slot 0x1d0; a stance-1 leash and `set_state` 13 are not
    /// modelled; the Hunt's Attack, a sub-action (`+0x60`), posts no notify 0x1fd); within it the
    /// target's place becomes the point. Ready, the unit faces the target (`FUN_141162db0`, at
    /// once for a unit without a turn time) and swings once its attack timer ran out (slot 0x100).
    /// A gate target (class 39, its parts' gaps) is not modelled.
    fn ready_to_attack(
        &mut self,
        walk: &mut Walk<'_>,
        s: &Strike<'_>,
    ) -> Result<Readiness, Unsupported> {
        let mover = walk
            .mover
            .as_deref_mut()
            .ok_or(Unsupported("an attacker without movement state"))?;
        let t = walk
            .world
            .get_known(self.target)?
            .ok_or(Unsupported("an Attack whose target is gone"))?;
        if 0 < t.player
            && !walk
                .world
                .sees_tile(s.own.player, t.position[0] as i32, t.position[1] as i32)
        {
            return Err(Unsupported(
                "readyToAttack on a target out of its player's sight",
            ));
        }
        let at = walk
            .world
            .get(walk.me.id)
            .map_or(walk.me.position, |p| p.position);
        let ground = [mover.stats.radius[0], mover.stats.radius[1]];
        let gap = footprint_gap(at, ground, t.position, t.radius);
        let moved = t.position != self.point;
        let far = (0..3).any(|i| HALF < (t.position[i] - self.point[i]).abs());
        let mobile = 0.0 < mover.stats.speed;
        if !moved || (!far && mobile) {
            return self.face(mover, at, t.position, s);
        }
        if !mobile {
            return Err(Unsupported(
                "readyToAttack of a unit that does not move (FUN_1412161a0)",
            ));
        }
        if gap < self.min_range {
            return Err(Unsupported(
                "an attack on a target closer than the least range",
            ));
        }
        if t.class == GATE {
            return Err(Unsupported("an attack on a gate (its parts' gaps)"));
        }
        if self.reach() < gap {
            if s.stance == 1 {
                return Err(Unsupported("an attack's leash test (stance 1)"));
            }
            if mover.stats.speed == 0.0 || !s.can_move {
                return Err(Unsupported(
                    "an attack whose target left the reach of a unit that cannot follow",
                ));
            }
            return Ok(Readiness::Away(gap));
        }
        self.point = t.position;
        self.face(mover, at, t.position, s)
    }

    /// readyToAttack's ready end (`0x140a801f2`): the unit at `at` faces its target at `to`
    /// (`FUN_141162db0`, [`crate::facing::face`]) and swings once its attack timer ran out.
    fn face(
        &self,
        mover: &mut Mover,
        at: [f32; 3],
        to: [f32; 3],
        s: &Strike<'_>,
    ) -> Result<Readiness, Unsupported> {
        face_target(mover, at, to)?;
        Ok(if *s.attack_timer <= 0.0 {
            Readiness::Ready
        } else {
            Readiness::Waiting
        })
    }

    /// `set_state(7)` (case 7): the swing. The unit takes the attack sprite (a second attack sprite,
    /// picked by an Object-stream draw, is not modelled) from its first frame (`FUN_1413b9f00`); a
    /// hit is pending; a reload shorter than the sprite speeds the animation up to fit it.
    fn set_swing(&mut self, walk: &mut Walk<'_>, s: &mut Strike<'_>) -> Result<(), Unsupported> {
        self.enter(SWING, walk, s)?;
        if self.arms.second_sprite.is_some() {
            return Err(Unsupported(
                "an attacker with a second attack sprite (its draw)",
            ));
        }
        let g = self
            .arms
            .sprite
            .ok_or(Unsupported("an attacker without an attack sprite"))?;
        set_sprite(s.sprite, g, s.sprite_rng);
        if let Some(sp) = s.sprite.as_mut() {
            sp.reset_frame();
        }
        self.pending = true;
        let length = f32::from(g.frames) * g.frame_duration;
        self.rest = s.reload - length;
        *s.anim_speed = Some(if self.rest < 0.0 && 0.0 < s.reload {
            length / s.reload
        } else {
            1.0
        });
        Ok(())
    }

    /// One update (slot 6, `0x140a82ae0`): the timer runs, the target is still there and where it
    /// was seen; then the state's step. Returns the action list's code: 0 under way, 5 done after
    /// its target died.
    pub fn update(&mut self, mut walk: Walk<'_>, s: &mut Strike<'_>) -> Result<u8, Unsupported> {
        self.timer += walk.dt;
        if self.state != KILLED && 2 < s.victim.state {
            // the target died before the attack ended (the update's start): a dead tree (class
            // 15) up to state 5 keeps a non-villager's attack going, and one of stance 0 that
            // moves would wait (set_state 5), neither modelled; a villager's unit AI keeps a dead
            // target of Gaia's (another's FUN_140c5e1e0 forgets, not modelled). The attack lets
            // the target go (slot 0x68 with none) and its point, posts no notify as the hunt's
            // sub-action (`+0x60`), ends (set_state 1) and returns 5
            if s.own.class != VILLAGER {
                return Err(Unsupported(
                    "an Attack of a unit other than a villager whose target died",
                ));
            }
            if s.victim.player != 0 {
                return Err(Unsupported(
                    "a villager's Attack whose player-owned target died (its unit AI forgets it)",
                ));
            }
            self.point = [NONE; 3];
            self.finish(&mut walk, s)?;
            return Ok(5);
        }
        let t = *walk
            .world
            .get_known(self.target)?
            .ok_or(Unsupported("an Attack whose target is gone"))?;
        if self.state != KILLED && (t.position[0] != self.seen[0] || t.position[1] != self.seen[1])
        {
            // the target moved: where it was seen follows (`+0xa4`..`+0xac`); one the unit's
            // player sees (the target's slot 0x1e0: 0xf) keeps the attack going, but an attack in
            // state 5 or 11 walks again (set_state 4). A target out of sight (or only explored,
            // with its master's fog flag) is not modelled.
            self.seen = t.position;
            if !walk
                .world
                .sees_tile(s.own.player, t.position[0] as i32, t.position[1] as i32)
            {
                return Err(Unsupported(
                    "an Attack whose target moved out of its player's sight",
                ));
            }
            if matches!(self.state, 5 | 11) {
                return Err(Unsupported(
                    "an Attack in state 5 or 11 whose target moved (set_state 4)",
                ));
            }
        }
        match self.state {
            MOVE => self.walk(walk, s, t.position, t.radius),
            READY => {
                match self.ready_to_attack(&mut walk.reborrow(), s)? {
                    Readiness::Ready => self.set_swing(&mut walk, s)?,
                    Readiness::Waiting => {}
                    Readiness::Away(gap) => self.follow(&mut walk, s, gap)?,
                }
                Ok(0)
            }
            SWING => self.swing(&mut walk, s),
            KILLED => {
                // case 1: once the swing's sprite looped (or there is none, or it does not
                // animate) the attack lets its target go and ends; the hunt reads code 5
                let done = s
                    .sprite
                    .is_none_or(|sp| sp.looped || sp.graphic.flags & 1 == 0);
                if !done {
                    return Ok(0);
                }
                self.finish(&mut walk, s)?;
                Ok(5)
            }
            _ => Err(Unsupported("an Attack in a state the slice does not model")),
        }
    }

    /// State 4 (cases 3, 4, 10): the `MoveTo` walks (`FUN_140a992b0`); a unit still farther than its
    /// reach goes on (code 0), one within it turns ready (`set_state` 6). The squared reach of a unit
    /// without a range is 0.01. A `MoveTo` that arrived returns 1: the list deletes it (its
    /// destructor, `0x140aa1f30`, stops the unit) and the unit turns ready (`set_state` 6).
    fn walk(
        &mut self,
        mut walk: Walk<'_>,
        s: &mut Strike<'_>,
        target: [f32; 3],
        radius: [f32; 2],
    ) -> Result<u8, Unsupported> {
        let code = match self.move_to {
            Some(MoveTo::Start) => {
                self.move_to = Some(MoveTo::Moving);
                let goal = Goal::Target(self.target);
                let state = start_walk(walk.reborrow(), goal, self.range, false)?;
                if state == MoveTo::Arrived {
                    // in range at its start: the MoveTo, not quiet, shows the idle sprite and
                    // clears `+0x7b8`
                    let g = self
                        .arms
                        .idle_sprite
                        .ok_or(Unsupported("an attacker without its standing sprite"))?;
                    set_sprite(s.sprite, g, s.sprite_rng);
                    if let Some(a) = s.anim_speed.as_mut() {
                        *a = 0.0;
                    }
                }
                self.move_to = Some(state);
                0
            }
            Some(MoveTo::Moving) => {
                // the MoveTo takes the Attack's `+0x61` (`0x140a824ac`), which the Hunt's
                // set_state(12) sets: a target that left its point asks for a path again
                if target_moved(&walk, self.target, true)? {
                    self.move_to = Some(MoveTo::Start);
                } else {
                    self.move_to = Some(walk_on(walk.reborrow())?);
                }
                0
            }
            Some(MoveTo::Arrived) => {
                self.move_to = None;
                walk.mover
                    .as_deref_mut()
                    .ok_or(Unsupported("an attacker without movement state"))?
                    .stop()?;
                self.ready(&mut walk, s)?;
                return Ok(0);
            }
            None => return Err(Unsupported("an Attack in state 4 without its MoveTo")),
            Some(MoveTo::Waiting | MoveTo::Failed) => {
                return Err(Unsupported(
                    "an Attack's waiting or failed MoveTo (states 2, 13)",
                ));
            }
        };
        let ground = walk
            .mover
            .as_ref()
            .map(|m| [m.stats.radius[0], m.stats.radius[1]])
            .ok_or(Unsupported("an attacker without movement state"))?;
        let at = walk
            .world
            .get(walk.me.id)
            .map_or(walk.me.position, |p| p.position);
        let reach = squared_reach(self.range);
        if reach.abs() <= EPSILON || reach < footprint_gap_squared(at, ground, target, radius) {
            return Ok(code);
        }
        self.ready(&mut walk, s)?;
        Ok(code)
    }

    /// State 7 (case 6). A melee swing counts down and lands its hit when its sprite reaches the
    /// firing frame (half its frames without one): the attack timer becomes the reload less the
    /// time to that frame (`FUN_141089820`) and the target takes the hit; a dead target ends the
    /// swing (`set_state` 2). A ranged unit's swing does not count down and fires at its firing
    /// frame (at once with frame 0): its missile ([`Strike::fire`]). After the hit, the swing's
    /// sprite looping makes the unit ready again (`set_state` 6).
    fn swing(&mut self, walk: &mut Walk<'_>, s: &mut Strike<'_>) -> Result<u8, Unsupported> {
        let ranged = self.arms.missile != -1;
        let Some(sprite) = *s.sprite else {
            return Err(Unsupported("a swing without the unit's sprite"));
        };
        if self.pending {
            if s.own.class == 51 || (!ranged && matches!(s.own.class, 35 | 64)) {
                return Err(Unsupported("a swing of class 35, 51 or 64"));
            }
            if ranged && self.arms.fire_frame < 0 {
                return Err(Unsupported("a ranged swing without a firing frame"));
            }
            let fire = if ranged {
                self.arms.fire_frame
            } else if self.arms.fire_frame < 1 {
                (f32::from(sprite.graphic.frames) * HALF) as u32 as i16
            } else {
                self.arms.fire_frame
            };
            if !ranged {
                self.swing -= walk.dt;
            }
            if fire != 0 && !sprite.reached(fire) {
                return Ok(0);
            }
            let frame = if self.arms.fire_frame == 0 && !ranged {
                sprite.graphic.frames / 2
            } else {
                self.arms.fire_frame
            };
            *s.attack_timer = s.reload - f32::from(frame) * sprite.graphic.frame_duration;
            if ranged {
                s.fire = Some(self.target);
            } else {
                let object = walk
                    .plan
                    .as_mut()
                    .map(|p| &mut *p.object)
                    .ok_or(Unsupported("a hit without the Object stream"))?;
                self.hit(s, object)?;
            }
            self.pending = false;
            if s.victim.hp <= 0.0 {
                self.enter(KILLED, walk, s)?;
                return Ok(0);
            }
            let rest = s.reload - f32::from(sprite.graphic.frames) * sprite.graphic.frame_duration;
            self.rest = rest;
            self.swing = rest;
            if rest < 0.0 {
                self.rest = 0.0;
            }
            return Ok(0);
        }
        if self.arms.interrupt_frame != -1 {
            return Err(Unsupported("a swing with an interrupt frame"));
        }
        if !sprite.looped && sprite.graphic.flags & 1 != 0 {
            return Ok(0);
        }
        // a reload longer than the sprite: at its last frame the unit waits with its idle attack
        // sprite (`+0x88`), else its standing one (slot 0x68), from its first frame when it
        // changes (`FUN_1413b9f00`), at speed 1
        if 0.0 < self.swing && sprite.reached(sprite.graphic.frames) {
            let g = self
                .arms
                .idle_attack_sprite
                .or(self.arms.idle_sprite)
                .ok_or(Unsupported("a swing's end without a sprite to wait with"))?;
            let changed = sprite.graphic.id != g.id;
            set_sprite(s.sprite, g, s.sprite_rng);
            if changed && let Some(sp) = s.sprite.as_mut() {
                sp.reset_frame();
            }
            *s.anim_speed = Some(1.0);
        }
        self.ready(walk, s)?;
        Ok(0)
    }

    /// The hit of a melee unit (slot 0x858 -> `FUN_141081020` -> slot 0x860,
    /// `FUN_1412115e0`): a unit with an area effect and a weapon range below 0.99 tramples
    /// ([`Attack::trample`]) unless its charge of type 3 is not its hit's (slot 0x2f0); else
    /// `FUN_141084230` ([`Attack::strike`]). Not held: the attack count (`+0x900`) and the
    /// damaged flag of masters 83, 293, 122 and 216 (`+0x683`); masters 122 and 123 under the
    /// game's `+0xc14` (`FUN_140c884c0`, 0 as the tick oracle holds it) would take the target at
    /// once.
    fn hit(&self, s: &mut Strike<'_>, object_rng: &mut Well512) -> Result<(), Unsupported> {
        let a = s.own.source;
        if 0.0 < a.area_effect
            && a.weapon_range < damage::BLAST_RANGE
            && (a.charge_type != 3 || damage::charge_hit(&a, s.victim))
        {
            return self.trample(s, object_rng);
        }
        self.strike(s, object_rng)
    }

    /// `FUN_141084230` with the unit's own place as the hit's origin ([`strike`]).
    fn strike(&self, s: &mut Strike<'_>, object_rng: &mut Well512) -> Result<(), Unsupported> {
        let hit = Hit {
            at: s.own.position,
            from_z: s.own.position[2],
            missile: false,
            chance: s.hit_chance,
        };
        let struck = strike(&mut s.own, s.victim, s.blasted, hit, object_rng)?;
        s.executed = struck.executed;
        Ok(())
    }

    /// `FUN_1412115e0`'s trample: the blast about the unit, past its target ([`damage::blast`]);
    /// the unit tells its unit AI the action executed (0x200, whatever its master); the 64-bit
    /// Object-stream draw against the hit chance; a hit the target's shield does not block
    /// (`FUN_14108d8a0`, on a target of any type) is its slot 0xe8 at the elevation factor times
    /// the hit's 1. A charge of type 1 or 3 the blast or the hit was for (`+0x8a8`) is emptied.
    fn trample(&self, s: &mut Strike<'_>, object_rng: &mut Well512) -> Result<(), Unsupported> {
        let (center, target) = (s.own.position, s.victim.id);
        let mut targets: Vec<&mut Victim> = s.blasted.iter_mut().collect();
        damage::blast(&mut s.own, center, Some(target), None, &mut targets, 1.0)?;
        s.executed = Some(target);
        let draw = object_rng.raw64();
        let r = (i64::from(draw as u32 & 0x7fff_ffff) * 100) / DRAW_SCALE;
        if r <= i64::from(s.hit_chance) {
            let factor =
                damage::elevation_factor(s.own.position[2], s.own.attack_elevation, s.victim);
            if !damage::shield(s.victim, false) {
                let weapons = s.own.weapons.clone();
                damage::take(s.victim, &mut s.own, &weapons, factor, 0.0)?;
            }
        }
        let a = &mut s.own.source;
        if a.charge_hit && matches!(a.charge_type, 1 | 3) {
            a.charge = 0.0;
        }
        Ok(())
    }
}

/// Where a hit comes from: the place it lands (`FUN_141084230`'s x, y, z: the attacker's for a
/// melee hit, the missile's for a missile's), the height of its origin record (`+0x8`, which the
/// elevation factor reads), whether a missile lands it (a shield of charge type 4 blocks it
/// instead of 5) and the attacker's hit chance (`+0x83e`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Hit {
    pub at: [f32; 3],
    pub from_z: f32,
    pub missile: bool,
    pub chance: i16,
}

/// What a hit did: its result (`+0x98` of a missile: 2 when a shield blocked it, else 1) and the
/// target of the executed action the attacker posts to its unit AI (notify 0x200 with 600).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Struck {
    pub result: u8,
    pub executed: Option<i32>,
}

/// `FUN_141084230`: a shield of the target (type 70, [`damage::shield`]) blocks the hit: the
/// target's unit AI gets notify 500 with the attacker, its hit points and its master's, and
/// nothing else happens (result 2). A unit of type 60 and up with an area effect blasts about the
/// hit's place with its target as the blast's main object ([`damage::blast`]) unless its charge
/// of type 3 is not its hit's, then empties that charge (slot 0x318 with 0). Else the unit tells
/// its unit AI the action executed (0x200, but for masters 73 and 559), then the 64-bit
/// Object-stream draw against the hit chance (a missile's master always hits: its missile,
/// `+0x83c`, is set); a hit is the target's slot 0xe8 ([`damage::take`]) at the elevation factor
/// (slot 0x1f0, from the origin's height) times the hit's 1. Not held: the last attack time
/// (`+0x648`), the alerts of another owner (slot 0x2e8) and the sounds.
pub(crate) fn strike(
    own: &mut Victim,
    victim: &mut Victim,
    blasted: &mut [Victim],
    hit: Hit,
    object_rng: &mut Well512,
) -> Result<Struck, Unsupported> {
    if victim.kind == 70 && damage::shield(victim, hit.missile) {
        let master_hp = own.armored.as_ref().map_or(0, |a| i32::from(a.master_hp));
        victim.damaged = Some((own.id, [own.id, own.hp as i32, master_hp]));
        return Ok(Struck {
            result: 2,
            executed: None,
        });
    }
    let a = own.source;
    let mut struck = Struck {
        result: 1,
        executed: None,
    };
    if 60 <= a.kind && 0.0 < a.area_effect && (a.charge_type != 3 || damage::charge_hit(&a, victim))
    {
        let main = victim.id;
        let mut targets: Vec<&mut Victim> = blasted.iter_mut().collect();
        targets.push(victim);
        damage::blast(own, hit.at, None, Some(main), &mut targets, 1.0)?;
        if a.charge_type == 3 {
            own.source.charge = 0.0;
        }
        return Ok(struck);
    }
    if !QUIET_SWINGS.contains(&own.master_id) {
        struck.executed = Some(victim.id);
    }
    let draw = object_rng.raw64();
    let r = (i64::from(draw as u32 & 0x7fff_ffff) * 100) / DRAW_SCALE;
    if i64::from(hit.chance) < r && a.missile < 0 {
        return Ok(struck);
    }
    let factor = damage::elevation_factor(hit.from_z, own.attack_elevation, victim);
    let weapons = own.weapons.clone();
    damage::take(victim, own, &weapons, factor, 0.0)?;
    Ok(struck)
}

/// The Attack constructor's sprites and frames (`FUN_140a7c210` from Hunt's `set_state` 12): the
/// task's proceeding sprite (else the master's attack sprite), the master's second attack sprite
/// and standing sprite, missile, firing and interrupting frames.
#[must_use]
pub fn arms(
    master: &crate::masters::Master,
    graphics: &std::collections::BTreeMap<i16, Graphic>,
    sprites: [i16; 4],
) -> Arms {
    let g = |id: i16| (0 <= id).then(|| graphics.get(&id).copied()).flatten();
    Arms {
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
) -> Result<Engagement, Unsupported> {
    if matches!(master.charge_type, 6 | 7) {
        return Err(Unsupported(
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
    })
}

/// The gap between two footprints, for an [`Engagement`].
#[must_use]
pub fn gaps(a: [f32; 3], a_radius: [f32; 2], b: [f32; 3], b_radius: [f32; 2]) -> (f32, f32) {
    let squared = footprint_gap_squared(a, a_radius, b, b_radius);
    (squared, footprint_gap(a, a_radius, b, b_radius))
}

/// `FUN_141162db0`: the unit at `at` turns to face `to`, at once for a unit without a turn time
/// ([`crate::facing::face`] with its sprites' facets); a unit that turns over time, or whose
/// sprites differ in their facets, is not modelled.
pub(crate) fn face_target(
    mover: &mut Mover,
    at: [f32; 3],
    to: [f32; 3],
) -> Result<(), Unsupported> {
    if !mover.stats.turns_at_once {
        return Err(Unsupported(
            "a unit that turns over time to face its target",
        ));
    }
    let facets = mover.stats.facets.ok_or(Unsupported(
        "a unit facing a target whose sprites differ in their facets",
    ))?;
    mover.angle = crate::facing::face([at[0], at[1]], mover.angle, [to[0], to[1]], Some(facets));
    Ok(())
}

/// The squared reach of an attack of `range` (`FUN_140a82ae0` at `0x140a83950`): the range squared,
/// or [`TOUCH`] for a range of at most [`NO_RANGE`].
fn squared_reach(range: f32) -> f32 {
    if NO_RANGE < range {
        range * range
    } else {
        TOUCH
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rangeless_attack_reaches_a_tenth_squared_in_f32() {
        // the image holds 0x3c23d70b (0.1f * 0.1f), one ULP above 0.01f
        assert_eq!(squared_reach(0.0).to_bits(), 0x3c23_d70b);
        assert_eq!(squared_reach(NO_RANGE).to_bits(), 0x3c23_d70b);
        assert_eq!(squared_reach(2.0), 4.0);
    }
}
