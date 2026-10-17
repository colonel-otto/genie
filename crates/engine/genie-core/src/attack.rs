//! The Attack action (`RGE_Action_Attack`, type 9, vtable `0x142ed47a8`) of a melee unit on an
//! object that does not fight back: a hunting villager's sub-action on a tree ([`crate::hunt`]).
//! Built away from its target, it walks there with a MoveTo sub-action ([`crate::move_to`]); in
//! reach it swings (state 7): the hit lands on the attack sprite's frame ([`crate::sprite`]), and
//! the unit waits out its reload between swings (state 6). A target that dies ends the swing
//! (state 2) and the action (set_state 1, code 5 to the hunt).
//!
//! 158041: constructor `FUN_140a7c210`, start (slot 5) `0x140a7f8c0`, update (slot 6)
//! `0x140a82ae0`, set_state (slot 16) `0x140a820c0`; the range test `FUN_140a7f9d0`
//! (`readyToAttack`); the hit (object slot 0x858, `FUN_14120c0a0` -> `FUN_141081020` -> slot 0x860
//! `FUN_1412115e0` -> `FUN_141084230` -> the target's slot 0xe8).

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
/// A MoveTo starts arrived within this gap (`DAT_142eb99cc`).
const ARRIVED: f32 = crate::image::attack::ARRIVED;
/// The gap below which the game takes a distance as zero (`DAT_142eb0c60`).
const EPSILON: f32 = crate::image::attack::EPSILON;
/// The point an attack on an object has none of (`DAT_14330ec20`).
const NONE: f32 = crate::image::attack::NONE;
/// The hit chance's scale (`0x7fffffff`, the low word of a 64-bit Object-stream draw).
const DRAW_SCALE: i64 = 0x7fff_ffff;
/// Elevation: a higher attacker hits at 1.25 (`DAT_142eb56a8`), a lower one at 0.75
/// (`DAT_142e7f388`), each plus its player's and the target's player's attributes (211/272,
/// 212/273).
const HIGHER: f32 = crate::image::attack::HIGHER;
const LOWER: f32 = crate::image::attack::LOWER;
/// The villager class (4): its hits leave what a static target holds.
const VILLAGER: i16 = 4;

/// The states the slice models: 1 done, 2 the target died, 4 on the way, 6 waiting to swing, 7
/// swinging.
pub const DONE: u8 = 1;
pub const KILLED: u8 = 2;
pub const MOVE: u8 = 4;
pub const READY: u8 = 6;
pub const SWING: u8 = 7;

/// The sprites and frames the Attack takes from its constructor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Arms {
    /// `+0x70`: the attack sprite (the hunt's task's proceeding sprite); `+0x78`, a second one
    /// the unit picks from; `+0x80`, the sprite it stands with when done.
    pub sprite: Option<Graphic>,
    pub second_sprite: Option<Graphic>,
    pub idle_sprite: Option<Graphic>,
    /// `+0x98`, the master's missile (`missile_id`, -1 for a melee unit); `+0x9a`, the frame that
    /// fires (`fire_missile_at_frame`); `+0x9c`, the frame that may interrupt
    /// (`interrupt_frame`), -1 for none.
    pub missile: i16,
    pub fire_frame: i16,
    pub interrupt_frame: i16,
}

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
    pub min_range: f32,
    /// `+0x38`..`+0x40`: the point it attacks; -1 for none.
    pub point: [f32; 3],
    /// `+0xa4`..`+0xac`: where it last saw its target.
    pub seen: [f32; 3],
    /// The MoveTo sub-action to the target, its range the attack's.
    pub move_to: Option<MoveTo>,
    /// `+0x9e`: a hit is still to land in this swing.
    pub pending: bool,
    /// `+0xb0` and `+0xb4`: the time of a swing past its sprite's length; a melee swing counts
    /// `+0xb0` down until the hit.
    pub swing: f32,
    pub rest: f32,
    pub arms: Arms,
}

/// What the Attack reads of its unit and its target when it starts.
pub struct Engagement {
    /// The gap between the two footprints squared (`FUN_14118f8c0`) and its root (`FUN_14118f680`).
    pub gap_squared: f32,
    pub gap: f32,
    /// The target's position.
    pub target_position: [f32; 3],
    /// The unit's top speed (slot 0x208), its master's weapon range with any bonus against the
    /// target (slot 0x238) and its least range.
    pub speed: f32,
    pub range: f32,
    pub min_range: f32,
    /// The unit AI's `canMoveToAttack` (`FUN_140c48420`): it has an order, or no order but a stance
    /// below 2.
    pub can_move: bool,
    /// The unit AI's stance (`+0x1a9`), whose 1 adds a leash test.
    pub stance: u8,
}

/// The target as a hit reads and changes it: a static object (type below 30), which has no
/// armor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Victim {
    pub id: i32,
    /// Its hit points (`+0x278`), what it holds (`+0x310`), its state and height.
    pub hp: f32,
    pub amount: f32,
    pub state: u8,
    pub z: f32,
    /// Its master's type; a hit on one of 30 or more (with armor) is not modelled.
    pub kind: u8,
    /// Flagged to sleep (`+0x641`, `+0x643`): a killing hit wakes it (`FUN_14119deb0` with 0).
    pub goto_sleep: bool,
    /// Its player's attributes 272 and 273, which a higher or lower attacker's hits add.
    pub elevation: [f32; 2],
    /// Its master's class and first storage amount (`+0x610`), which a gatherer reads.
    pub class: i16,
    pub storage: f32,
    /// Its master's decay (`+0x620`), which a hunt's decay stop reads (`FUN_140ac44b0`).
    pub decay: f32,
}

/// What the Attack's update reads of its unit besides its movement, and changes.
pub struct Strike<'a> {
    /// The unit's active sprite and animation speed (`+0x7b8`), None while the slice does not
    /// know them.
    pub sprite: &'a mut Option<ActiveSprite>,
    pub anim_speed: &'a mut Option<f32>,
    /// `+0x91c`, the time until it may attack again (slot 0x100: at most 0); the game's world
    /// time when it may again (`+0x920`, slot 0x248) is not held.
    pub attack_timer: &'a mut f32,
    /// Slot 0x218 (`FUN_141086c90`): the master's reload time (`speed_of_attack`) with the
    /// unit's reload stinger and powerup.
    pub reload: f32,
    /// The master's weapons (class, amount; `+0x810`, `+0x818`), its hit chance (`+0x83e`) and
    /// class; the unit's player's attributes 211 and 212 (elevation).
    pub weapons: &'a [(i16, i16)],
    pub hit_chance: i16,
    pub class: i16,
    pub elevation: [f32; 2],
    /// The unit's height (`+0x5a8`).
    pub z: f32,
    pub victim: &'a mut Victim,
    /// The Sprite stream, which a sprite with a random first frame draws from; the hit draws
    /// from the walk's Object stream.
    pub sprite_rng: &'a mut Well512,
    /// The unit's master's id: masters 73 and 559 post no executed action.
    pub master_id: i16,
    /// The unit's player, whose sight of a target that moved the update reads (its slot 0x1e0).
    pub player: usize,
    /// The target of a swing this update, whose executed action (notify 0x200 with 600 and the
    /// target) the unit posts to its unit AI.
    pub executed: Option<i32>,
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

/// What readyToAttack answers for a target in reach.
enum Readiness {
    Ready,
    Waiting,
}

impl Attack {
    /// The constructor (`FUN_140a7c210`, from Hunt's set_state 12 with the master's weapon range,
    /// least range, missile and frame) and the start (slot 5 with 1, `0x140a7f8c0`): the target is
    /// the object itself (the area-effect and gate cases are not reached by a villager); a range of
    /// 0 becomes slot 0x238's. The start runs the range test: a target in reach becomes the
    /// point and the unit faces it, a unit out of reach that may move turns to state 4; a unit that
    /// may move turns to state 4 again in the start, each time with a new MoveTo (whose teardown
    /// stops the unit). The unit's `+0x900` (attack count) is cleared and its sprites set; the
    /// slice holds neither.
    pub fn start(
        target: i32,
        e: &Engagement,
        arms: Arms,
        mover: &mut Mover,
    ) -> Result<Self, Unsupported> {
        let mut a = Attack {
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
            // (FUN_141162db0, at once for a unit without a turn time; its angle is not held)
            a.point = e.target_position;
            if !mover.stats.turns_at_once {
                return Err(Unsupported(
                    "an attacker that turns over time to face its target",
                ));
            }
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
            a.set_move(e, mover)?;
        }
        // the start's own test of the speed and the unit AI (slots 0x208 and 0x1d0): state 4
        a.set_move(e, mover)?;
        Ok(a)
    }

    /// How far the unit reaches: the range, half a tile more without one.
    fn reach(&self) -> f32 {
        if self.range <= NO_RANGE {
            self.range + HALF
        } else {
            self.range
        }
    }

    /// set_state(4) (`0x140a820c0` case 4): the sub-actions go (a MoveTo's teardown stops the unit,
    /// slot 0x5d0), the range is slot 0x238's again, and a MoveTo to the target with that range
    /// starts (`FUN_140aa0df0`, start `FUN_140aa2100`): arrived within 0.01, else in state 3 for a
    /// unit with speed.
    fn set_move(&mut self, e: &Engagement, mover: &mut Mover) -> Result<(), Unsupported> {
        if self.move_to.take().is_some() {
            mover.stop()?;
        }
        self.state = MOVE;
        self.range = e.range;
        self.timer = 0.0;
        if e.gap <= ARRIVED {
            return Err(Unsupported("an Attack's MoveTo that starts arrived"));
        }
        self.move_to = Some(MoveTo::Start);
        Ok(())
    }

    /// The teardown of the sub-actions (`FUN_140a73fc0` on the action list): a MoveTo stops the
    /// unit.
    pub fn tear_down(&mut self, mover: Option<&mut Mover>) -> Result<(), Unsupported> {
        if self.move_to.take().is_some()
            && let Some(m) = mover
        {
            m.stop()?;
        }
        Ok(())
    }

    /// The start of every set_state (`0x140a820c0`): the sub-actions go (a MoveTo's teardown stops
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

    /// set_state(1) (case 1): the unit's animation runs at speed 1 and it takes the sprite it
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

    /// set_state(6) (case 6): the range test; in reach and ready, the swing (set_state 7).
    fn ready(&mut self, walk: &mut Walk<'_>, s: &mut Strike<'_>) -> Result<(), Unsupported> {
        self.enter(READY, walk, s)?;
        match self.ready_to_attack(walk, s)? {
            Readiness::Ready => self.set_swing(walk, s),
            Readiness::Waiting => Ok(()),
        }
    }

    /// readyToAttack (`FUN_140a7f9d0`) on a target that does not move: in reach (the gap at most
    /// the reach, at least the least range) the target's place becomes the attack's point, the
    /// unit faces it (`FUN_141162db0`, at once for a unit without a turn time) and is ready when
    /// its attack timer ran out (slot 0x100). A target out of reach or too close is not modelled
    /// here.
    fn ready_to_attack(
        &mut self,
        walk: &Walk<'_>,
        s: &Strike<'_>,
    ) -> Result<Readiness, Unsupported> {
        let mover = walk
            .mover
            .as_deref()
            .ok_or(Unsupported("an attacker without movement state"))?;
        let t = walk
            .world
            .get_known(self.target)?
            .ok_or(Unsupported("an Attack whose target is gone"))?;
        let at = walk
            .world
            .get(walk.me.id)
            .map_or(walk.me.position, |p| p.position);
        let ground = [mover.stats.radius[0], mover.stats.radius[1]];
        let gap = footprint_gap(at, ground, t.position, t.radius);
        if gap < self.min_range {
            return Err(Unsupported(
                "an attack on a target closer than the least range",
            ));
        }
        if self.reach() < gap {
            return Err(Unsupported("an attack whose target left its reach"));
        }
        self.point = t.position;
        if !mover.stats.turns_at_once {
            return Err(Unsupported(
                "an attacker that turns over time to face its target",
            ));
        }
        Ok(if *s.attack_timer <= 0.0 {
            Readiness::Ready
        } else {
            Readiness::Waiting
        })
    }

    /// set_state(7) (case 7): the swing. The unit takes the attack sprite (a second attack sprite,
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
            return Err(Unsupported(
                "an Attack whose target died before its swing ended",
            ));
        }
        let t = walk
            .world
            .get_known(self.target)?
            .ok_or(Unsupported("an Attack whose target is gone"))?
            .clone();
        if self.state != KILLED && (t.position[0] != self.seen[0] || t.position[1] != self.seen[1])
        {
            // the target moved: where it was seen follows (`+0xa4`..`+0xac`); one the unit's
            // player sees (the target's slot 0x1e0: 0xf) keeps the attack going, but an attack in
            // state 5 or 11 walks again (set_state 4). A target out of sight (or only explored,
            // with its master's fog flag) is not modelled.
            self.seen = t.position;
            if !walk
                .world
                .sees_tile(s.player, t.position[0] as i32, t.position[1] as i32)
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
                if let Readiness::Ready = self.ready_to_attack(&walk, s)? {
                    self.set_swing(&mut walk, s)?;
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

    /// State 4 (cases 3, 4, 10): the MoveTo walks (`FUN_140a992b0`); a unit still farther than its
    /// reach goes on (code 0), one within it turns ready (set_state 6). The squared reach of a unit
    /// without a range is 0.01.
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
                self.move_to = Some(start_walk(walk.reborrow(), goal, self.range, false)?);
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
                return Err(Unsupported("an Attack whose MoveTo arrived"));
            }
            None => return Err(Unsupported("an Attack in state 4 without its MoveTo")),
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
    /// swing (set_state 2). After the hit, the swing's sprite looping makes the unit ready again
    /// (set_state 6).
    fn swing(&mut self, walk: &mut Walk<'_>, s: &mut Strike<'_>) -> Result<u8, Unsupported> {
        if self.arms.missile != -1 {
            return Err(Unsupported("a ranged unit's swing (its missile)"));
        }
        let Some(sprite) = *s.sprite else {
            return Err(Unsupported("a swing without the unit's sprite"));
        };
        if self.pending {
            if matches!(s.class, 35 | 51 | 64) {
                return Err(Unsupported("a swing of class 35, 51 or 64"));
            }
            let fire = if self.arms.fire_frame < 1 {
                (f32::from(sprite.graphic.frames) * HALF) as u32 as i16
            } else {
                self.arms.fire_frame
            };
            self.swing -= walk.dt;
            if fire != 0 && !sprite.reached(fire) {
                return Ok(0);
            }
            let frame = if self.arms.fire_frame == 0 {
                sprite.graphic.frames / 2
            } else {
                self.arms.fire_frame
            };
            *s.attack_timer = s.reload - f32::from(frame) * sprite.graphic.frame_duration;
            let object = walk
                .plan
                .as_mut()
                .map(|p| &mut *p.object)
                .ok_or(Unsupported("a hit without the Object stream"))?;
            self.hit(s, object)?;
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
        if 0.0 < self.swing && sprite.reached(sprite.graphic.frames) {
            return Err(Unsupported(
                "a swing longer than its reload (the sprite's reset)",
            ));
        }
        self.ready(walk, s)?;
        Ok(0)
    }

    /// The hit of a melee unit on a static object (slot 0x858 -> `FUN_141081020` -> slot 0x860 ->
    /// `FUN_141084230`): a 64-bit Object-stream draw against the hit chance; a hit costs the target
    /// the attacker's weapons' sum times the elevation factor (a static object has no armor, slot
    /// 0xf0 `FUN_14118cab0`); below 1 hit point it has 0 and wakes, and a non-villager's hit takes
    /// what it holds (slot 0xe8 `FUN_14118efd0`). Before the draw the unit tells its unit AI the
    /// action executed (0x200, but for masters 73 and 559). The attack count, the last attack
    /// time (`+0x648`) and the sounds are not held.
    fn hit(&self, s: &mut Strike<'_>, object_rng: &mut Well512) -> Result<(), Unsupported> {
        if 30 <= s.victim.kind {
            return Err(Unsupported("a hit on an object with armor"));
        }
        if !QUIET_SWINGS.contains(&s.master_id) {
            s.executed = Some(s.victim.id);
        }
        let v = &mut *s.victim;
        let draw = object_rng.raw64();
        let r = (i64::from(draw as u32 & 0x7fff_ffff) * 100) / DRAW_SCALE;
        if i64::from(s.hit_chance) < r {
            return Ok(());
        }
        let factor = if v.z < s.z {
            s.elevation[0] + HIGHER + v.elevation[0]
        } else if s.z < v.z {
            s.elevation[1] + LOWER + v.elevation[1]
        } else {
            1.0
        };
        let mut damage = 0.0f32;
        for &(_, amount) in s.weapons {
            damage += f32::from(amount) * factor;
        }
        v.hp -= damage;
        if v.hp < 1.0 {
            v.hp = 0.0;
            v.goto_sleep = false;
            if s.class != VILLAGER {
                v.amount = 0.0;
            }
        }
        Ok(())
    }
}

/// The Attack constructor's sprites and frames (`FUN_140a7c210` from Hunt's set_state 12): the
/// task's proceeding sprite (else the master's attack sprite), the master's second attack sprite
/// and standing sprite, missile, firing and interrupting frames.
pub fn arms(
    master: &crate::masters::Master,
    graphics: &std::collections::BTreeMap<i16, Graphic>,
    sprites: [i16; 4],
) -> Arms {
    let g = |id: i16| (0 <= id).then(|| graphics.get(&id).copied()).flatten();
    Arms {
        sprite: g(sprites[1]).or(g(master.attack_sprite)),
        second_sprite: g(master.attack_sprite2),
        idle_sprite: g(master.standing_sprite),
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
        target_position: them.position,
        speed,
        range: master.weapon_range,
        min_range: master.min_range,
        can_move: ai.is_some_and(|ai| ai.current_order != -1 || ai.stance <= 1),
        stance: ai.map_or(0, |ai| ai.stance),
    })
}

/// The gap between two footprints, for an [`Engagement`].
pub fn gaps(a: [f32; 3], a_radius: [f32; 2], b: [f32; 3], b_radius: [f32; 2]) -> (f32, f32) {
    let squared = footprint_gap_squared(a, a_radius, b, b_radius);
    (squared, footprint_gap(a, a_radius, b, b_radius))
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
