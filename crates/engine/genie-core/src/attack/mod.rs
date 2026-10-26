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

use crate::damage::Victim;
use crate::math::rng::Well512;
use crate::move_to::{MoveTo, MoveWait, Walk};
use crate::sprite::{ActiveSprite, Graphic};

mod constructor;
mod hits;
mod ready_to_attack;
mod sprites;
mod states;
mod update;
mod walk_and_swing;
mod work_slots;

pub use self::constructor::{arms, arms_looking, engagement, gaps};
use self::hits::{AfterHit, after_hit, blast_so_far};
pub(crate) use self::hits::{Hit, strike};
#[cfg(test)]
use self::ready_to_attack::in_range;
use self::sprites::{NO_OBJECT_STREAM, set_attack_timer};
pub use self::sprites::{SwingSprites, swing_sprites};
pub(crate) use self::sprites::{face_target, reset_sprite, set_sprite, show_arrived, volleys};
use self::update::squared_reach;
pub use self::work_slots::{Mark, Wielder, Worked};
#[cfg(test)]
use crate::movement::Mover;
#[cfg(test)]
use crate::{GenieError, damage};

/// The Attack action's type, which word 9 adds.
pub const ATTACK: i16 = 9;

/// A range at most this counts as none (`DAT_142eb5690`): the range test widens the gap by
/// `-0.5` (`DAT_142eb6e58`) and the reach by `0.5` (`DAT_142e7f384`).
const NO_RANGE: f32 = 0.1;
const HALF: f32 = 0.5;
/// The squared reach of an attack without a range (`DAT_142ed48e4`): `0x3c23d70b`, which is
/// `0.1f * 0.1f`, one ULP above `0.01f`.
const TOUCH: f32 = crate::math::image::attack::TOUCH;
/// A `MoveTo` starts arrived within this gap (`DAT_142eb99cc`).
const ARRIVED: f32 = crate::math::image::attack::ARRIVED;
/// The gap below which the game takes a distance as zero (`DAT_142eb0c60`).
const EPSILON: f32 = crate::math::image::attack::EPSILON;
/// The point an attack on an object has none of (`DAT_14330ec20`).
const NONE: f32 = crate::math::image::attack::NONE;
/// The villagers' class, whose `MoveTo` waits once when the motion step gives up.
const VILLAGER_CLASS: i16 = 4;
/// The milliseconds of a second the reload world time counts (`FUN_141089820`).
const RELOAD_MS: f32 = crate::math::image::attack::RELOAD_MS;
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
/// State 11: a human player's unit that reached where its target was seen without seeing it
/// walks onto that point (a `MoveTo` without a range).
const GOTO_POINT: u8 = 11;
/// The least range of a unit that walks onto the point where it saw its target
/// (`DAT_14330eb8c`, [`GOTO_POINT`]).
const SEEN_RANGE: f32 = crate::math::image::attack::SEEN_RANGE;
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
    /// The point its `MoveTo` of states 5 and 11 walks to (that `MoveTo`'s `+0x38`..`+0x40`):
    /// where it saw its target when it took the state; −1 for none.
    pub goto: [f32; 3],
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
    /// `+0x9f`, `was_same_owner` (saved): its target was its unit's player's when the
    /// constructor (`FUN_140a7c210`) took it; the update's ally test spares such a livestock or
    /// farm target ([`Attack::update`]).
    pub same_owner: bool,
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
    /// A unit that cannot move out of reach of its target: its start's `set_state(13)` is the
    /// caller's ([`Attack::fail_at_start`]).
    pub failed: bool,
    /// readyToAttack failed it (`set_state(13)`) before the start's `set_state(4)` replaced that:
    /// the unit's animation runs at speed 1 (`+0x7b8`), the caller's; the standing sprite
    /// `set_state(13)` shows is the start's already.
    pub failed_first: bool,
}

/// How the start ([`Attack::begin`]) leaves an Attack.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Begun {
    /// Its last `MoveTo` started arrived.
    arrived: bool,
    /// It ends in `set_state(6)`, the caller's.
    ready: bool,
    /// It ends in `set_state(13)`, the caller's.
    failed: bool,
    /// readyToAttack's `set_state(13)` came before the start's `set_state(4)`.
    failed_first: bool,
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
    /// The unit AI's `state_position` (`+0x1ac`, `+0x1b0`), the leash's centre.
    pub state_position: [f32; 2],
    /// The world's time (`FUN_1415300d0`), which a facing over time reads.
    pub world_time: u32,
    /// The unit's master, of which 1704 and 1706 do not turn to face their target.
    pub master: i16,
    /// The target's master's class, whose 39 (a gate) readyToAttack measures by its parts.
    pub target_class: i16,
    /// The target is the unit's player's (the constructor's `+0x9f`, [`Attack::same_owner`]).
    pub same_owner: bool,
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
    /// The unit AI's `state_position` (`+0x1ac`, `+0x1b0`), the centre of stance 1's leash.
    pub state_position: [f32; 2],
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
    /// The swing's own sprites ([`Attack::set_swing`]): its terrain look's attack sprite and its
    /// master's special graphic.
    pub swing_sprites: SwingSprites,
}

/// readyToAttack's gap for a range up to [`NO_RANGE`]: half a tile less (`DAT_142eb6e58`).
const SHORT_RANGE_GAP: f32 = crate::math::image::attack::SHORT_RANGE_GAP;

/// readyToAttack's leash of stance 1 lets a target this much past the range from the unit AI's
/// `state_position` (`DAT_14330ebb8`, read by `ADDSS` at `0x140a80051`).
const LEASH_SLACK: f32 = crate::math::image::attack::LEASH_SLACK;

/// The masters readyToAttack does not turn to face their target (`0x6744`, `0x6746` raw).
const MASTER_1704: i16 = 1704;
const MASTER_1706: i16 = 1706;

/// The masters whose swings post no executed action (`0x60e5`, `0x62cb` raw).
const QUIET_SWINGS: [i16; 2] = [73, 559];

/// The livestock (58, `0x3a`) and farm (49, `0x31`) classes: the ally test spares an Attack on
/// one that was its unit's player's when the Attack took it ([`Attack::same_owner`]).
const LIVESTOCK: i16 = 58;
const FARM: i16 = 49;
/// A building's master type (`'P'`), whose main building (`+0xbb8`) an area effect attacks.
const BUILDING: u8 = 80;
/// The class an Attack does not take as its target (`-0xe70` raw, [`Attack::set_target`]).
const CLASS_60: i16 = 60;

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
/// The trees' class, a dead one of which up to state 5 keeps another unit's attack going.
const TREE: i16 = 15;

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

#[cfg(test)]
pub(crate) mod tests;
