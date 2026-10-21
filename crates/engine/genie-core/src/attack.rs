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
use crate::move_to::{
    Follow, Goal, MoveTo, MoveWait, Walk, follow_path_end, start_walk_or_give_up, target_moved,
    walk_or_give_up,
};
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
/// The villagers' class, whose `MoveTo` waits once when the motion step gives up.
const VILLAGER_CLASS: i16 = 4;
/// The milliseconds of a second the reload world time counts (`FUN_141089820`).
const RELOAD_MS: f32 = crate::image::attack::RELOAD_MS;
/// The hit chance's scale (`0x7fffffff`, the low word of a 64-bit Object-stream draw).
const DRAW_SCALE: i64 = 0x7fff_ffff;
/// The states the slice models: 1 done, 2 the target died, 4 on the way, 6 waiting to swing, 7
/// swinging, 13 failed.
pub const DONE: u8 = 1;
/// State 2: the target died.
pub const KILLED: u8 = 2;
/// State 4: on the way.
pub const MOVE: u8 = 4;
/// State 6: waiting to swing.
pub const READY: u8 = 6;
/// State 7: swinging.
pub const SWING: u8 = 7;
/// State 13 ("Failed"): the target went out of sight.
pub const FAILED: u8 = 13;
/// The waiting state 5 ("Goto2"), a `MoveTo` to where the target was seen.
const GOTO_SEEN: u8 = 5;
/// The predators' class (10), whose unit AI's slot 0x1f0 a failed Attack calls.
const PREDATOR: i16 = 10;
/// The boar's unit-AI type, whose slot 0x1f0 (`FUN_140c16d50`) sends it home.
const BOAR_AI: u32 = 4;
/// The lion's unit-AI type, whose slot 0x1f0 (`FUN_140c16d60`) does nothing.
const LION_AI: u32 = 5;

/// The sprites and frames the Attack takes from its constructor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Arms {
    /// `+0x68`: the sprite its `MoveTo` shows (its constructor's argument: the hunt's walking
    /// sprite, the task's moving one; else the master's running one, slot 0x7d0, else its
    /// walking one, slot 0x7c8).
    pub move_sprite: Option<Graphic>,
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
    /// The second target (`+0x28`, its id at `+0x34`): the work command's Attack takes its
    /// target as this too (`FUN_140a7a190`, slot 0x178), −1 for none.
    pub target2: i32,
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
    /// Its `MoveTo`'s wait after the motion step gave up.
    pub move_wait: MoveWait,
    /// `+0x9e`: a hit is still to land in this swing.
    pub pending: bool,
    /// `+0xb0` and `+0xb4`: the time of a swing past its sprite's length; a melee swing counts
    /// `+0xb0` down until the hit.
    pub swing: f32,
    /// `+0xb4`: the rest after the swing's sprite.
    pub rest: f32,
    /// The sprites and frames from its constructor.
    pub arms: Arms,
    /// `+0x60`, `subActionValue`: the action is another's sub-action (the Hunt's) and posts no
    /// notifies; a unit's own Attack posts them to its unit AI ([`Strike::posted`]).
    pub sub_action: bool,
    /// `+0x61`, `mTargetMovedState`, which its `MoveTo` takes: a target that leaves the
    /// `MoveTo`'s point makes it ask for a path at once (the Hunt's `set_state` 12 sets it).
    pub repaths: bool,
    /// `+0x68`, the sprite its `MoveTo` shows, is the master's running one (a unit's own
    /// Attack's, slot 0x7d0): the `MoveTo`'s start sets the unit's running speed. The Hunt's
    /// (its task's sprite) leaves the speed.
    pub runs: bool,
}

/// A unit's own Attack as the attack command made it ([`Attack::command`]).
#[derive(Clone, Debug, PartialEq)]
pub struct Commanded {
    /// The attack.
    pub attack: Attack,
    /// Its last `MoveTo` started arrived.
    pub arrived: bool,
    /// The notifies its start posted (kind and three parameters).
    pub posted: Vec<(i32, [i32; 3])>,
    /// A unit that cannot move in reach of its target: its start's `set_state(6)` (READY) is the
    /// caller's ([`Attack::enter_ready`]), with what the swing reads.
    pub ready: bool,
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
    /// The world's time (`FUN_1415300d0`), which a facing over time reads.
    pub world_time: u32,
}

/// What the Attack's update reads of its unit besides its movement, and changes.
#[derive(Debug)]
pub struct Strike<'a> {
    /// The unit's active sprite and animation speed (`+0x7b8`), None while the slice does not
    /// know them.
    pub sprite: &'a mut Option<ActiveSprite>,
    /// The unit's animation speed (`+0x7b8`), None while the slice does not know it.
    pub anim_speed: &'a mut Option<f32>,
    /// Its master's standing sprite (`+0x470`), which slot 0x68 shows for no sprite
    /// (`FUN_141158620`).
    pub standing: Option<Graphic>,
    /// Its master has a volley missile (`+0x8d4`) or a charge projectile (`+0x928`) its player
    /// has: a hit without a missile is a volley too (`FUN_14120c0a0`), not the melee hit.
    pub volleys: bool,
    /// `+0x91c`, the time until it may attack again (slot 0x100: at most 0).
    pub attack_timer: &'a mut f32,
    /// `mReloadWorldtime` (`+0x920`, slot 0x248): the world time it may again.
    pub reload_worldtime: &'a mut u32,
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
    /// The notifies a unit's own Attack posts to its unit AI this update (slot 0x328: kind and
    /// three parameters), the caller's.
    pub posted: Vec<(i32, [i32; 3])>,
    /// The unit has a unit AI whose order queue is empty (`FUN_140c564a0` is 0).
    pub queue_empty: bool,
    /// The world's time (`FUN_1415300d0`), which a facing over time reads.
    pub world_time: u32,
    /// The unit AI's order (`+0x3c`, `FUN_140c4cc20`), None without a unit AI.
    pub ai_order: Option<i32>,
    /// The unit AI's factory type, None without a unit AI.
    pub ai_type: Option<u32>,
    /// The unit's player is a computer (`player+0x10 == 3`, `FUN_141659cf0`).
    pub computer: bool,
    /// `FUN_14120ea30` for a building: it can attack (class 51 or 54, a missile, or a firepower
    /// above 1); true for the others.
    pub armed: bool,
    /// `FUN_1412161a0`: a volley is under way (`next_volley`, `+0xa04`, not 0).
    pub volley_under_way: bool,
    /// The update let its target go (slot 0x68 with none: its slot 0x180), the caller's.
    pub let_go: Option<i32>,
    /// The update made the unit AI forget its target (`FUN_140c5e1e0`), the caller's.
    pub forgets: bool,
    /// The update sent a boar home at its next idle check (its unit AI's slot 0x1f0,
    /// `FUN_140c16d50`: `+0x1fd`), the caller's.
    pub sends_home: bool,
}

/// readyToAttack's gap for a range up to [`NO_RANGE`]: half a tile less (`DAT_142eb6e58`).
const SHORT_RANGE_GAP: f32 = crate::image::attack::SHORT_RANGE_GAP;

/// The masters readyToAttack does not turn to face their target (`0x6744`, `0x6746` raw).
const MASTER_1704: i16 = 1704;
const MASTER_1706: i16 = 1706;

/// The masters whose swings post no executed action (`0x60e5`, `0x62cb` raw).
const QUIET_SWINGS: [i16; 2] = [73, 559];

/// `FUN_141089820`: the attack timer becomes `t` and the reload world time the world's time plus
/// `t` in whole milliseconds (slot 0x248).
fn set_attack_timer(s: &mut Strike<'_>, t: f32) {
    *s.attack_timer = t;
    *s.reload_worldtime = s.world_time.wrapping_add((t * RELOAD_MS) as i64 as u32);
}

/// `FUN_14120c0a0`'s test of `master`: it has a volley missile or a charge projectile its player
/// has (`masters`), so its hits go through its volley even without a missile.
pub(crate) fn volleys(master: &crate::masters::Master, masters: &crate::masters::Masters) -> bool {
    let has = |id: Option<i16>| id.and_then(|id| masters.get(id)).is_some();
    has(master.volley_missile) || has(master.charge_projectile)
}

/// Slot 0x68 (`FUN_141217000`): the unit's sprite becomes `g` unless it is already; a new sprite
/// starts a new instance (`FUN_1413b2ef0`).
/// [`Attack::arrived_sprite`] for a caller that holds the unit's sprite and animation speed: the
/// idle sprite of `arms` (slot 0x68), the speed `+0x7b8` cleared.
pub(crate) fn show_arrived(
    arms: &Arms,
    sprite: &mut Option<ActiveSprite>,
    anim_speed: &mut Option<f32>,
    rng: &mut Well512,
) -> Result<(), Unsupported> {
    let g = arms
        .idle_sprite
        .ok_or(Unsupported("an attacker without its standing sprite"))?;
    set_sprite(sprite, g, rng);
    if let Some(a) = anim_speed.as_mut() {
        *a = 0.0;
    }
    Ok(())
}

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

/// The units in the way the motion step told the unit AI of (its slot 0x38 with
/// [`crate::unit_ai::IN_THE_WAY`], [`crate::movement::Mover::in_the_way`]), in the order of the
/// Attack's posts; the walker is in state 2, which the unit's own post (slot 0x328) asks for.
pub(crate) fn in_the_way(walk: &mut Walk<'_>, s: &mut Strike<'_>) {
    if let Some(m) = walk.mover.as_deref_mut() {
        s.posted.extend(
            m.in_the_way
                .drain(..)
                .map(|id| (crate::unit_ai::IN_THE_WAY, [id, -1, -1])),
        );
    }
}

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
            target2: -1,
            state: 0,
            timer: 0.0,
            range: e.range,
            min_range: e.min_range,
            point: [NONE; 3],
            seen: e.target_position,
            move_to: None,
            move_wait: MoveWait::default(),
            pending: false,
            swing: 0.0,
            rest: 0.0,
            arms,
            sub_action: true,
            repaths: true,
            runs: false,
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
            let _ = face_target(mover, e.position, e.target_position, e.world_time)?;
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

    /// A unit's own Attack (the attack command, `FUN_141081a80`): the constructor
    /// (`FUN_140a7c210` with sub-action 0: the master's weapon range and least range, its
    /// sprites, missile and frames, its `MoveTo`'s the running one, slot 0x7d0, or the walking
    /// one: the running speed either way; `+0x61` clear) and the start's range test (slot 5,
    /// `0x140a7f8c0`, readyToAttack): a player's target the unit's player neither sees nor
    /// explored with its master's fog flag (`visible` false) makes a unit of stance 0 that may
    /// move follow where it was seen (`set_state` 5: a `MoveTo` to that point, which starts
    /// unless it arrived, not modelled), any other unit stops (`set_state` 13, not modelled); a
    /// target it sees takes the start's test of [`Attack::start`] (in reach the point and the
    /// facing, else `set_state` 4 after notify 0x1fd). Then a unit that may move turns to state 4
    /// with a new `MoveTo` (the teardown of the old one stops the unit); one that cannot is not
    /// modelled.
    ///
    /// # Errors
    ///
    /// What the start does not model.
    pub fn command(
        target: i32,
        e: &Engagement,
        arms: Arms,
        visible: bool,
        target_owner: usize,
        mover: &mut Mover,
    ) -> Result<Commanded, Unsupported> {
        let mut a = Self {
            target,
            target2: -1,
            state: 0,
            timer: 0.0,
            range: e.range,
            min_range: e.min_range,
            point: [NONE; 3],
            seen: e.target_position,
            move_to: None,
            move_wait: MoveWait::default(),
            pending: false,
            swing: 0.0,
            rest: 0.0,
            arms,
            sub_action: false,
            repaths: false,
            runs: true,
        };
        let movable = e.speed != 0.0 && e.can_move;
        let mut posted = Vec::new();
        if 0 < target_owner && !visible {
            if e.stance != 0 || e.speed <= 0.0 || !e.can_move {
                return Err(Unsupported(
                    "an attack on a target out of sight by a unit that cannot follow (set_state 13)",
                ));
            }
            // set_state(5): the MoveTo to where the target was seen (FUN_140aa0d30) starts
            // (state 3) unless the unit's footprint is within reach of the point already
            if e.gap <= ARRIVED {
                return Err(Unsupported(
                    "an attack's MoveTo to the target's last place that starts arrived",
                ));
            }
            a.state = 5;
            a.range = e.range;
            a.timer = 0.0;
            a.move_to = Some(MoveTo::Start);
        } else {
            if a.min_range > e.gap {
                return Err(Unsupported(
                    "an attack on a target closer than the least range",
                ));
            }
            if a.reach() >= e.gap {
                a.point = e.target_position;
                let _ = face_target(mover, e.position, e.target_position, e.world_time)?;
            } else {
                if e.stance == 1 {
                    return Err(Unsupported("an attack's leash test (stance 1)"));
                }
                posted.push((0x1fd, [0; 3]));
                if movable {
                    let _ = a.set_move(e, mover)?;
                }
            }
        }
        // the start's end (0x140a7f8c0): a unit that cannot move goes ready (set_state 6) when
        // readyToAttack reached its ready end (in reach), else stops (13)
        if !movable {
            if a.state == 5 || a.reach() < e.gap {
                return Err(Unsupported(
                    "an attack command of a unit that cannot move out of reach (set_state 13)",
                ));
            }
            return Ok(Commanded {
                attack: a,
                arrived: false,
                posted,
                ready: true,
            });
        }
        let arrived = a.set_move(e, mover)?;
        Ok(Commanded {
            attack: a,
            arrived,
            posted,
            ready: false,
        })
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
    ) -> Result<(), Unsupported> {
        self.ready(&mut walk, s)
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

    /// A unit's own Attack whose target died (the update's start, not quiet: `+0x60` 0): its unit
    /// AI forgets the target (`FUN_140c5e1e0`; a villager keeps a Gaia target), the target goes
    /// (slot 0x68 with none) and the point; a human player's unit whose unit AI has an order and
    /// an empty queue (or no unit AI) posts 0x1fa with 600 (the action completed), any other
    /// with a unit AI whose queue is empty 0x1fb with 600 and 0x202; it ends (`set_state` 1) and
    /// returns 5, which keeps it in the list. A dead tree (class 15) up to state 5, which keeps
    /// a non-villager at it, and a mover of stance 0 that waits instead (`set_state` 5) are not
    /// modelled.
    fn lost(&mut self, walk: &mut Walk<'_>, s: &mut Strike<'_>) -> Result<u8, Unsupported> {
        if s.victim.class == 15 && s.own.class != VILLAGER && s.victim.state <= 5 {
            return Err(Unsupported("an Attack on a dead tree (state up to 5)"));
        }
        if s.ai_order.is_some() && (s.victim.player != 0 || s.own.class != VILLAGER) {
            s.forgets = true;
        }
        s.let_go = Some(self.target);
        self.target = -1;
        self.point = [NONE; 3];
        let human = s.own.player != 0 && !s.computer;
        if human && s.ai_order.is_none_or(|o| o != -1 && s.queue_empty) {
            let speed = walk.mover.as_ref().map_or(0.0, |m| m.stats.speed);
            if s.stance == 0 && 0.0 < speed && s.own.class != VILLAGER {
                return Err(Unsupported(
                    "an Attack whose target died waiting for another (set_state 5)",
                ));
            }
            s.posted.push((0x1fa, [600, 0, 0]));
        } else if s.ai_order.is_some() && s.queue_empty {
            s.posted.push((0x1fb, [600, 0, 0x202]));
        }
        self.finish(walk, s)?;
        Ok(5)
    }

    /// The update's answer to a target out of sight ([`Attack::update`]): the target goes (slot
    /// 0x68 with none), the attack fails (`set_state` 13, as 1), the unit AI forgets the target
    /// (`FUN_140c5e1e0`), a predator's unit AI's slot 0x1f0 runs (the boar's sends it home at its
    /// next idle check, `sends_home`; the lion's does nothing) and a unit's own Attack whose unit AI's queue is empty posts 0x1f9 with 600;
    /// returns 3, which deletes it.
    fn out_of_sight(&mut self, walk: &mut Walk<'_>, s: &mut Strike<'_>) -> Result<u8, Unsupported> {
        s.let_go = Some(self.target);
        self.target = -1;
        self.finish_in(FAILED, walk, s)?;
        s.forgets = s.ai_order.is_some();
        if s.own.class == PREDATOR {
            match s.ai_type {
                Some(BOAR_AI) => s.sends_home = true,
                Some(LION_AI) | None => {}
                Some(_) => {
                    return Err(Unsupported(
                        "a predator's failed Attack whose unit AI is not a boar's or a lion's (slot 0x1f0)",
                    ));
                }
            }
        }
        if !self.sub_action && s.queue_empty {
            s.posted.push((crate::unit_ai::ACTION_FAILED, [600, 0, 0]));
        }
        Ok(3)
    }

    /// `set_state(1)` (case 1): the unit's animation runs at speed 1 and it takes the sprite it
    /// stands with.
    fn finish(&mut self, walk: &mut Walk<'_>, s: &mut Strike<'_>) -> Result<(), Unsupported> {
        self.finish_in(DONE, walk, s)
    }

    /// `set_state` cases 1, 13 and 14: as [`Attack::finish`], in `state`.
    fn finish_in(
        &mut self,
        state: u8,
        walk: &mut Walk<'_>,
        s: &mut Strike<'_>,
    ) -> Result<(), Unsupported> {
        self.enter(state, walk, s)?;
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
        // readyToAttack posts 0x1fd first for a unit's own Attack
        if !self.sub_action {
            s.posted.push((0x1fd, [0; 3]));
        }
        self.enter(MOVE, walk, s)?;
        if self.moving(s.own.source.weapon_range, gap) {
            self.arrived_sprite(s)?;
        }
        Ok(())
    }

    /// readyToAttack (`FUN_140a7f9d0`) on an object: a target of a player (not Gaia) out of the
    /// unit's player's sight (its slot 0x1e0 neither 0xf nor 0x80 with its master's fog flag) is
    /// not modelled (`set_state` 5 or 13). A unit that moves (a master of type 30 and up with a
    /// speed, `FUN_140fdd080`; the slice reads the unit's top speed) whose target stands on the
    /// attack's point (`FUN_140a7bbf0`) or within half a tile of it on each axis
    /// (`FUN_140a7bd10`) is ready at once; so is a unit that does not move with such a target in
    /// its range (the gap half a tile less for a range up to 0.1) and not under its least range
    /// (none up to 0.1), or with a volley under way (`FUN_1412161a0`); else the reach decides:
    /// the target's gap
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
        let (tx, ty) = (t.position[0] as i32, t.position[1] as i32);
        if 0 < t.player
            && !walk.world.sees_tile(s.own.player, tx, ty)
            && !(s.victim.fog_visibility != 0 && walk.world.explored(s.own.player, tx, ty)?)
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
        if !far {
            // a unit that does not move: ready with the target in its range (the gap half a tile
            // less for a range up to 0.1) and not under its least range (none up to 0.1), or
            // with a volley under way (FUN_1412161a0)
            let near = if self.range <= NO_RANGE {
                gap + SHORT_RANGE_GAP
            } else {
                gap
            };
            let in_range =
                near <= self.range && (self.min_range <= NO_RANGE || self.min_range <= near);
            if in_range || s.volley_under_way {
                return self.face(mover, at, t.position, s);
            }
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
    /// (`FUN_141162db0`, [`face_target`]; masters 1704 and 1706 skip it); one not facing it yet
    /// waits; one facing it swings once its attack timer ran out.
    fn face(
        &self,
        mover: &mut Mover,
        at: [f32; 3],
        to: [f32; 3],
        s: &Strike<'_>,
    ) -> Result<Readiness, Unsupported> {
        if !matches!(s.own.master_id, MASTER_1704 | MASTER_1706)
            && !face_target(mover, at, to, s.world_time)?
        {
            return Ok(Readiness::Waiting);
        }
        Ok(if *s.attack_timer <= 0.0 {
            Readiness::Ready
        } else {
            Readiness::Waiting
        })
    }

    /// `set_state(7)` (case 7): the swing. The unit takes the attack sprite (a second attack sprite,
    /// picked by an Object-stream draw, is not modelled; without one slot 0x68 shows its master's
    /// standing sprite) from its first frame (`FUN_1413b9f00`); a hit is pending; a reload shorter
    /// than the sprite speeds the animation up to fit it.
    fn set_swing(&mut self, walk: &mut Walk<'_>, s: &mut Strike<'_>) -> Result<(), Unsupported> {
        self.enter(SWING, walk, s)?;
        if self.arms.second_sprite.is_some() {
            return Err(Unsupported(
                "an attacker with a second attack sprite (its draw)",
            ));
        }
        let g = self.arms.sprite.or(s.standing).ok_or(Unsupported(
            "an attacker without an attack or standing sprite",
        ))?;
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

    /// Slot 9 (`FUN_140a841d0`) on `target`, the Attack's own target (an attacker with an area
    /// effect would take a building's main building): one in states 4 to 7 or 11 keeps it as it
    /// is. The range test of the other states (and the new target's) is not modelled.
    pub const fn keeps(&self, target: i32) -> Result<bool, Unsupported> {
        if target == self.target && (matches!(self.state, 4..=7) || self.state == 11) {
            return Ok(true);
        }
        Err(Unsupported(
            "an Attack's slot 9 on a target out of its states 4 to 7 and 11",
        ))
    }

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
    ) -> Result<(u8, bool), Unsupported> {
        self.timer += dt;
        if !armed {
            self.state = DONE;
            return Ok((1, true));
        }
        if self.state != DONE {
            return Err(Unsupported("an Attack without a target in a state but 1"));
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
    ) -> Result<Option<(i32, [i32; 3])>, Unsupported> {
        if self.move_to.is_some() {
            return Err(Unsupported("a building's Attack with a MoveTo"));
        }
        *anim_speed = Some(1.0);
        let idle = self
            .arms
            .idle_sprite
            .ok_or(Unsupported("an Attack's unit without a standing sprite"))?;
        set_sprite(sprite, idle, rng);
        Ok((!self.sub_action).then_some((0x1fb, [600, self.target, 0])))
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
            if !self.sub_action {
                return self.lost(&mut walk, s);
            }
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
            // player sees (the target's slot 0x1e0, `FUN_1416635e0` at its tile: 0xf), or only
            // explored (0x80) with its master's fog flag (`+0x605`), keeps the attack going, but
            // an attack in state 5 or 11 walks again (set_state 4). One out of sight keeps an
            // attack in a state but 4 and 5 going; a player's unit without a unit AI, or whose
            // unit AI has an order and an empty queue, waits (set_state 5, not modelled; one in
            // state 5 keeps going); any other fails ([`Attack::out_of_sight`])
            self.seen = t.position;
            let (tx, ty) = (t.position[0] as i32, t.position[1] as i32);
            let seen = walk.world.sees_tile(s.own.player, tx, ty)
                || (s.victim.fog_visibility != 0 && walk.world.explored(s.own.player, tx, ty)?);
            if !seen && matches!(self.state, MOVE | GOTO_SEEN) {
                if s.own.player == 0 || s.ai_order.is_some_and(|o| o == -1 || !s.queue_empty) {
                    return self.out_of_sight(&mut walk, s);
                }
                if self.state != GOTO_SEEN {
                    return Err(Unsupported(
                        "an Attack whose target moved out of its player's sight waiting (set_state 5)",
                    ));
                }
            }
            if seen && matches!(self.state, GOTO_SEEN | 11) {
                return Err(Unsupported(
                    "an Attack in state 5 or 11 whose target moved (set_state 4)",
                ));
            }
        }
        // a target inside another (+0x2f0): the attack ends (set_state 1) and returns 4, which
        // keeps it in the list; a unit's own Attack (not quiet, +0x60) of a unit whose unit AI's
        // order queue is empty posts 0x1fb with 600 and the target
        if s.victim.inside {
            self.finish(&mut walk, s)?;
            if !self.sub_action && s.queue_empty {
                s.posted.push((0x1fb, [600, self.target, 0]));
            }
            return Ok(4);
        }
        // a building that cannot attack (FUN_14120ea30) ends it (set_state 1), its own Attack
        // posting 0x1fb with 600 and the target, and returns 1: the list deletes it
        if !s.armed {
            self.finish(&mut walk, s)?;
            if !self.sub_action {
                s.posted.push((0x1fb, [600, self.target, 0]));
            }
            return Ok(1);
        }
        match self.state {
            // case 0: the unit's animation stops; the list deletes the Attack
            DONE => {
                *s.anim_speed = Some(0.0);
                Ok(1)
            }
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
                if self.sub_action {
                    self.finish(&mut walk, s)?;
                    return Ok(5);
                }
                // a unit's own Attack (not quiet, `+0x60` 0): the target goes (slot 0x68 with
                // none), it posts 0x1fb with 600 and 0x202, ends (set_state 1) and returns 2,
                // which keeps it in the list
                s.let_go = Some(self.target);
                self.target = -1;
                s.posted.push((0x1fb, [600, 0, 0x202]));
                self.finish(&mut walk, s)?;
                Ok(2)
            }
            // case 0xc: a failed attack posts 0x1f9 with 600 every update (quiet or not), the
            // unit's animation stops and the update returns 3
            FAILED => {
                s.posted.push((crate::unit_ai::ACTION_FAILED, [600, 0, 0]));
                *s.anim_speed = Some(0.0);
                Ok(3)
            }
            _ => Err(Unsupported("an Attack in a state the slice does not model")),
        }
    }

    /// The `MoveTo`'s answer to the motion step giving up ([`MoveWait::give_up`]).
    const fn give_up(&mut self, s: &Strike<'_>) {
        let waits = !s.computer && s.own.class == VILLAGER_CLASS;
        self.move_to = Some(self.move_wait.give_up(waits, s.stance));
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
                // case 3 after a wait: `+0x6c` goes from 1 to 2 as it asks again
                if self.move_wait.state == 1 {
                    self.move_wait.state = 2;
                }
                // the MoveTo's sprite sets the unit's speed: a running one the running speed
                if self.runs
                    && let Some(m) = walk.mover.as_deref_mut()
                {
                    m.stats.speed = m.stats.run_speed;
                }
                self.move_to = Some(MoveTo::Moving);
                let goal = Goal::Target(self.target);
                let walked = start_walk_or_give_up(walk.reborrow(), goal, self.range, false)?;
                in_the_way(&mut walk, s);
                // the MoveTo's set_state(4) shows its sprite (`+0x58`, the Attack's `+0x68`)
                if walked.is_none_or(|w| w == MoveTo::Moving)
                    && let Some(g) = self.arms.move_sprite
                {
                    set_sprite(s.sprite, g, s.sprite_rng);
                }
                let Some(state) = walked else {
                    // the motion step gave up in the update set_state(4) ran; a MoveTo that fails
                    // at once (set_state 13, not quiet: 0x140aa29e0) shows the idle sprite and
                    // clears `+0x7b8` as an arrived one does
                    self.give_up(s);
                    if self.move_to == Some(MoveTo::Failed) {
                        let g = self
                            .arms
                            .idle_sprite
                            .ok_or(Unsupported("an attacker without its standing sprite"))?;
                        set_sprite(s.sprite, g, s.sprite_rng);
                        if let Some(a) = s.anim_speed.as_mut() {
                            *a = 0.0;
                        }
                    }
                    return Ok(0);
                };
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
                let follow = target_moved(&walk, self.target, self.repaths)?;
                if let Follow::PathEnd(p) = follow {
                    follow_path_end(&mut walk, p)?;
                }
                if follow.again() {
                    if self.repaths {
                        // with `+0x61` the MoveTo only turns to state 3 (`LAB_140aa3a59`)
                        self.move_to = Some(MoveTo::Start);
                    } else {
                        // without it the MoveTo tells the unit (notify 0x1fc), turns to state 3
                        // and runs the motion step (`LAB_140aa320c`): moving on, it stays in
                        // state 3
                        s.posted.push((0x1fc, [0; 3]));
                        let walked = walk_or_give_up(walk.reborrow())?;
                        in_the_way(&mut walk, s);
                        match walked {
                            Some(MoveTo::Moving) => self.move_to = Some(MoveTo::Start),
                            Some(other) => self.move_to = Some(other),
                            None => self.give_up(s),
                        }
                    }
                } else {
                    let walked = walk_or_give_up(walk.reborrow())?;
                    in_the_way(&mut walk, s);
                    match walked {
                        Some(m) => self.move_to = Some(m),
                        None => self.give_up(s),
                    }
                }
                0
            }
            // case 2: the timer runs by dt; at its end the MoveTo asks again (set_state 3)
            Some(MoveTo::Waiting) => {
                self.move_wait.timer += walk.dt;
                if self.move_wait.delay <= self.move_wait.timer {
                    self.move_wait.timer = 0.0;
                    self.move_to = Some(MoveTo::Start);
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
            // case 3, the quiet MoveTo's failure (its update returns 3 without a notify):
            // set_state(13); a unit's own Attack (`+0x60` 0) posts 0x1f9 with 600; the update
            // returns 3 and the list removes the Attack
            Some(MoveTo::Failed) => {
                self.move_to = None;
                self.finish_in(FAILED, &mut walk, s)?;
                if !self.sub_action {
                    s.posted.push((crate::unit_ai::ACTION_FAILED, [600, 0, 0]));
                }
                return Ok(3);
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
            set_attack_timer(
                s,
                s.reload - f32::from(frame) * sprite.graphic.frame_duration,
            );
            // slot 0x858 (FUN_14120c0a0): the volley, but for a unit without a missile, volley
            // missile or charge projectile, whose hit is FUN_141081020's melee one
            if ranged || s.volleys {
                s.fire = Some(self.target);
            } else {
                if matches!(s.own.master_id, 527 | 528 | 1104 | 2133..=2135) {
                    return Err(Unsupported(
                        "a self-destructing attacker's hit (masters 527, 528, 1104, 2133..2135)",
                    ));
                }
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
            from_z: Some(s.own.position[2]),
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
/// melee hit, the missile's for a missile's), the height the elevation factor compares with the
/// target's (the attacker's slot 0x1f0: a combat object's, `FUN_14120d780`, its origin record's
/// `+0x8` when the record names an owner that has its master, else its own; None for a missile
/// whose record names no owner, `FUN_14113e550`: a factor of 1), whether a missile lands it (a
/// shield of charge type 4 blocks it instead of 5) and the attacker's hit chance (`+0x83e`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Hit {
    pub at: [f32; 3],
    pub from_z: Option<f32>,
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
    let factor = hit.from_z.map_or(1.0, |z| {
        damage::elevation_factor(z, own.attack_elevation, victim)
    });
    let weapons = own.weapons.clone();
    damage::take(victim, own, &weapons, factor, 0.0)?;
    Ok(struck)
}

/// The Attack constructor's sprites and frames (`FUN_140a7c210`): the `MoveTo`'s sprite `moving`
/// (the hunt's walking sprite from Hunt's `set_state` 12, the task's moving one from the task
/// factory, none from the attack command), else the master's running sprite (slot 0x7d0), else
/// its walking one (slot 0x7c8; the unit's `+0x8a9` overrides are not held); the task's
/// proceeding sprite (else the master's attack sprite), the master's second attack sprite and
/// standing sprite, missile, firing and interrupting frames.
#[must_use]
pub fn arms(
    master: &crate::masters::Master,
    graphics: &std::collections::BTreeMap<i16, Graphic>,
    sprites: [i16; 4],
    moving: i16,
) -> Arms {
    let g = |id: i16| (0 <= id).then(|| graphics.get(&id).copied()).flatten();
    Arms {
        move_sprite: g(moving)
            .or_else(|| g(master.run_sprite))
            .or_else(|| g(master.move_sprite)),
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
        world_time,
    })
}

/// The gap between two footprints, for an [`Engagement`].
#[must_use]
pub fn gaps(a: [f32; 3], a_radius: [f32; 2], b: [f32; 3], b_radius: [f32; 2]) -> (f32, f32) {
    let squared = footprint_gap_squared(a, a_radius, b, b_radius);
    (squared, footprint_gap(a, a_radius, b, b_radius))
}

/// `FUN_141162db0`: the unit at `at` turns to face `to` with its sprites' facets, at once for a
/// unit without a turn time ([`crate::facing::face`]), else a facet a turn time
/// ([`crate::facing::turn_toward`] by the world time); returns whether it faces. The facets are
/// its current sprite's ([`Mover::facets`]).
pub(crate) fn face_target(
    mover: &mut Mover,
    at: [f32; 3],
    to: [f32; 3],
    world_time: u32,
) -> Result<bool, Unsupported> {
    let facets = mover.facets().ok_or(Unsupported(
        "a unit facing a target whose sprites differ in their facets",
    ))?;
    let (at, to) = ([at[0], at[1]], [to[0], to[1]]);
    if mover.stats.turn_time == 0.0 {
        mover.angle = crate::facing::face(at, mover.angle, to, Some(facets));
        return Ok(true);
    }
    let (angle, last, faces) = crate::facing::turn_toward(
        at,
        mover.angle,
        to,
        Some(facets),
        mover.stats.turn_time,
        mover.last_turn,
        world_time,
    );
    (mover.angle, mover.last_turn) = (angle, last);
    Ok(faces)
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

    /// 425906646's town centre 27171 (player 6) in update 6224: its Attack lost the boar 29826
    /// and the town centre cannot attack (no missile, a firepower of at most 1): the Attack ends
    /// (`set_state(1)`, the standing sprite at speed 1) and posts 0x1fb with 600 and its target,
    /// −1.
    #[test]
    fn a_disarmed_building_s_targetless_attack_ends_with_its_notify() {
        let standing = Graphic {
            id: 418,
            frames: 1,
            facets: 1,
            speed_multiplier: 1.0,
            frame_duration: 0.0,
            replay_delay: 0.0,
            flags: 0,
        };
        let mut a = Attack {
            target: -1,
            target2: -1,
            state: READY,
            timer: 0.0,
            range: 6.0,
            min_range: 0.0,
            point: [NONE; 3],
            seen: [NONE; 3],
            move_to: None,
            move_wait: MoveWait::default(),
            pending: false,
            swing: 0.0,
            rest: 0.0,
            arms: Arms {
                move_sprite: None,
                sprite: None,
                second_sprite: None,
                idle_sprite: Some(standing),
                idle_attack_sprite: None,
                missile: -1,
                fire_frame: 0,
                interrupt_frame: -1,
            },
            sub_action: false,
            repaths: false,
            runs: false,
        };
        let mut anim = Some(0.5);
        assert_eq!(
            a.clone().update_targetless(0.028, true, &mut anim),
            Err(Unsupported("an Attack without a target in a state but 1"))
        );
        assert_eq!(a.update_targetless(0.028, false, &mut anim), Ok((1, true)));
        assert_eq!(a.state, DONE);
        let (mut sprite, mut rng) = (None, Well512::default());
        let posted = a.disarmed(&mut sprite, &mut anim, &mut rng).unwrap();
        assert_eq!(posted, Some((0x1fb, [600, -1, 0])));
        assert_eq!((anim, sprite.map(|s| s.graphic.id)), (Some(1.0), Some(418)));
        // the Hunt's Attack, a sub-action, posts nothing
        a.sub_action = true;
        assert_eq!(a.disarmed(&mut sprite, &mut anim, &mut rng).unwrap(), None);
    }
}
