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
    /// A unit that cannot move out of reach of its target: its start's `set_state(13)` is the
    /// caller's ([`Attack::fail_at_start`]).
    pub failed: bool,
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

/// `FUN_1413b9f00` on the object's sprite list (`+0x148`, genie's: the unit's one sprite): the
/// first instance of graphic `g` (a node's `+0x8`) goes back to its first frame (its slot 9 with
/// `0x7fff`, [`ActiveSprite::reset_frame`]); false when the list holds none.
pub(crate) const fn reset_sprite(sprite: &mut Option<ActiveSprite>, g: Graphic) -> bool {
    match sprite.as_mut() {
        Some(sp) if sp.graphic.id == g.id => {
            sp.reset_frame();
            true
        }
        _ => false,
    }
}

pub(crate) fn set_sprite(sprite: &mut Option<ActiveSprite>, g: Graphic, rng: &mut Well512) {
    if sprite.is_some_and(|s| s.graphic.id == g.id) {
        return;
    }
    *sprite = Some(ActiveSprite::start(g, rng));
}

/// A building's master type (`'P'`), whose main building (`+0xbb8`) an area effect attacks.
const BUILDING: u8 = 80;
/// The class an Attack does not take as its target (`-0xe70` raw, [`Attack::set_target`]).
const CLASS_60: i16 = 60;

/// An object as the Attack's slot 9 ([`Attack::work`]) and slot 0x68 ([`Attack::set_target`])
/// read it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mark {
    /// The object.
    pub id: i32,
    /// Its master's type (80 a building).
    pub kind: u8,
    /// Its master's class.
    pub class: i16,
    /// Its state (`+0x640`).
    pub state: u8,
    /// Its place (decoded `x`, `y`, `z`).
    pub position: [f32; 3],
    /// Its master's radii on the ground, which the footprint gap (`FUN_14118f680`) reads.
    pub radius: [f32; 2],
}

/// What the Attack's slot 9 ([`Attack::work`]) reads of its unit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Wielder {
    /// Its master's `area_effect_range` (`+0x82c`).
    pub area_effect: f32,
    /// Its top speed (slot 0x208).
    pub speed: f32,
    /// Its place.
    pub position: [f32; 3],
    /// Its master's radii on the ground.
    pub radius: [f32; 2],
}

/// What the Attack's slot 9 or slot 0x68 leaves to its caller.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Worked {
    /// Slot 9 took the target (its answer, 1).
    pub kept: bool,
    /// Slot 0x68 took another target: the unit's attack count (`+0x900`) clears.
    pub clears_count: bool,
    /// The old target, whose slot 0x180 lets the unit go when the world still holds it.
    pub lets_go: Option<i32>,
    /// The new target, whose slot 0x178 takes the unit with the action's type (9).
    pub takes: Option<i32>,
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
            if a.state == 5 {
                return Err(Unsupported(
                    "an attack command of a unit that cannot move to where its target was seen",
                ));
            }
            let failed = a.reach() < e.gap;
            return Ok(Commanded {
                attack: a,
                arrived: false,
                posted,
                ready: !failed,
                failed,
            });
        }
        let arrived = a.set_move(e, mover)?;
        Ok(Commanded {
            attack: a,
            arrived,
            posted,
            ready: false,
            failed: false,
        })
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
    ) -> Result<(), Unsupported> {
        self.state = FAILED;
        *anim_speed = Some(1.0);
        let idle = self
            .arms
            .idle_sprite
            .ok_or(Unsupported("an Attack's unit without a standing sprite"))?;
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

    /// Slot 0x38 (`FUN_140a82ac0`) as the unit's stop calls it: `set_state(1)` (the
    /// sub-actions go, a `MoveTo`'s teardown stopping the unit and its animation; the animation
    /// runs at speed 1 and the unit takes the sprite it stands with, [`Attack::finish`]); its
    /// answer is true (the caller's), so the stop keeps the action list.
    ///
    /// # Errors
    ///
    /// An attacker walking without movement state, or without a standing sprite.
    pub(crate) fn halt(
        &mut self,
        mover: Option<&mut Mover>,
        anim_speed: &mut Option<f32>,
        sprite: &mut Option<ActiveSprite>,
        rng: &mut Well512,
    ) -> Result<(), Unsupported> {
        if self.move_to.take().is_some() {
            mover
                .ok_or(Unsupported("an attacker without movement state"))?
                .stop()?;
            if let Some(a) = anim_speed.as_mut() {
                *a = 0.0;
            }
        }
        self.state = DONE;
        *anim_speed = Some(1.0);
        let idle = self
            .arms
            .idle_sprite
            .ok_or(Unsupported("an Attack's unit without a standing sprite"))?;
        set_sprite(sprite, idle, rng);
        Ok(())
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
        match self.ready_to_attack(&mut walk.reborrow(), s, false)? {
            Readiness::Ready => self.set_swing(walk, s),
            Readiness::Waiting => Ok(()),
            Readiness::Away(gap) => self.follow(walk, s, gap),
        }
    }

    /// readyToAttack's answer to a target out of reach of a unit that may follow it (its notify
    /// 0x1fd posted): `set_state(4)`, the `MoveTo` after it with the range of slot 0x238 (the
    /// master's weapon range).
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

    /// readyToAttack (`FUN_140a7f9d0`) on an object; `skip_ready` is its third argument (1 from
    /// `set_state(7)`'s range clamp: no ready-at-once test). A target of a player (not Gaia) out
    /// of the unit's player's sight (its slot 0x1e0 neither 0xf nor 0x80 with its master's fog
    /// flag) fails the attack (`set_state` 13) unless the unit AI's stance (`+0x1a9`) is 0, the
    /// top speed above 0 (`COMISS`/`JBE`) and canMoveToAttack 1, whose wait (`set_state` 5, a
    /// `MoveTo` to where it was seen) is not modelled. A unit that moves (a master of type 30 and
    /// up with a speed, `FUN_140fdd080`; the slice reads the unit's top speed, above 0 just when
    /// the master's is) whose target stands on the attack's point (`FUN_140a7bbf0`) or within
    /// half a tile of it on each axis (`FUN_140a7bd10`: a NaN difference is not within) is ready
    /// at once; so is a unit that does not move with such a target in its range ([`in_range`]),
    /// or with a volley under way (`FUN_1412161a0`). Else the reach decides, by the target's gap
    /// (`FUN_14118f680`): below the least range (`COMISS`/`JA`) a unit's own Attack posts 0x1fe
    /// (−1, −1, −1); a unit with a top speed (`UCOMISS`: not 0) whose unit AI may move to attack
    /// (slot 0x1d0) waits, any other fails (`set_state` 13). Beyond the reach (the range, half a
    /// tile more without one) a unit's own Attack posts 0x1fd; a stance-1 leash (the unit AI's
    /// point, `+0x1ac`, `+0x1b0`) is not modelled; a unit that may move follows the target
    /// (`set_state` 4, [`Readiness::Away`]), any other fails. Within it the target's place becomes
    /// the point and the unit faces it ([`Attack::face`]). A gate target (class 39, its parts'
    /// gaps) is not modelled.
    fn ready_to_attack(
        &mut self,
        walk: &mut Walk<'_>,
        s: &mut Strike<'_>,
        skip_ready: bool,
    ) -> Result<Readiness, Unsupported> {
        let t = *walk
            .world
            .get_known(self.target)?
            .ok_or(Unsupported("an Attack whose target is gone"))?;
        let speed = walk
            .mover
            .as_deref()
            .ok_or(Unsupported("an attacker without movement state"))?
            .stats
            .speed;
        let (tx, ty) = (t.position[0] as i32, t.position[1] as i32);
        if 0 < t.player
            && !walk.world.sees_tile(s.own.player, tx, ty)
            && !(s.victim.fog_visibility != 0 && walk.world.explored(s.own.player, tx, ty)?)
        {
            if s.stance == 0 && 0.0 < speed && s.can_move {
                return Err(Unsupported(
                    "readyToAttack's wait for a target out of sight (set_state 5)",
                ));
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
            .ok_or(Unsupported("an attacker without movement state"))?;
        let ground = [mover.stats.radius[0], mover.stats.radius[1]];
        let gap = footprint_gap(at, ground, t.position, t.radius);
        let near = if self.range <= NO_RANGE {
            gap + SHORT_RANGE_GAP
        } else {
            gap
        };
        let moved = t.position != self.point;
        let far = (0..3)
            .any(|i| crate::fmath::below_or_unordered(HALF, (t.position[i] - self.point[i]).abs()));
        let mobile = 30 <= s.own.kind && 0.0 < speed;
        if !skip_ready
            && (!moved
                || (!far
                    && (mobile
                        || in_range(near, self.range, self.min_range)
                        || s.volley_under_way)))
        {
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
        if t.class == GATE {
            return Err(Unsupported("an attack on a gate (its parts' gaps)"));
        }
        if self.reach() < gap {
            if !self.sub_action {
                s.posted.push((0x1fd, [0; 3]));
            }
            if s.stance == 1 {
                return Err(Unsupported("an attack's leash test (stance 1)"));
            }
            if speed == 0.0 || !s.can_move {
                return self.fail(walk, s);
            }
            return Ok(Readiness::Away(gap));
        }
        self.point = t.position;
        self.face(mover, at, t.position, s)
    }

    /// readyToAttack's `set_state(13)` (as [`Attack::finish_in`]); it answers 0.
    fn fail(&mut self, walk: &mut Walk<'_>, s: &mut Strike<'_>) -> Result<Readiness, Unsupported> {
        self.finish_in(FAILED, walk, s)?;
        Ok(Readiness::Waiting)
    }

    /// readyToAttack's ready end (`0x140a801f2`): the unit at `at` faces its target at `to`
    /// (`FUN_141162db0`, [`face_target`]; masters 1704 and 1706 skip it); one not facing it yet
    /// waits; one facing it swings once its attack timer ran out (slot 0x100, `FUN_141082d40`:
    /// `COMISS` with 0 then `SETBE`, so a NaN timer has run out).
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
        Ok(
            if crate::fmath::at_most_or_unordered(*s.attack_timer, 0.0) {
                Readiness::Ready
            } else {
                Readiness::Waiting
            },
        )
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
        // FUN_1413b9f00 on the unit's sprite list with its sprite (`+0x140`)
        if let Some(current) = s.sprite.map(|sp| sp.graphic) {
            let _ = reset_sprite(s.sprite, current);
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

    /// Slot 9 (`FUN_140a841d0`) on `target`, the Attack's own target, for a caller that holds no
    /// more of the unit and the target than the id (the work command's `keep_attack`): one in
    /// states 4 to 7 or 11 keeps it as it is. The rest of the slot (the main building of an area
    /// effect's building target, the range test, the new target) is [`Attack::work`]'s, which
    /// needs the unit's and the target's values.
    pub const fn keeps(&self, target: i32) -> Result<bool, Unsupported> {
        if target == self.target && (matches!(self.state, 4..=7) || self.state == 11) {
            return Ok(true);
        }
        Err(Unsupported(
            "an Attack's slot 9 on a target out of its states 4 to 7 and 11",
        ))
    }

    /// Slot 9 (`FUN_140a841d0`, `RGE_Action_Attack::work`) on `target`: none answers false. A
    /// unit whose master has an area effect (`+0x82c` above 0: `COMISS`/`JBE`) takes a building's
    /// (type 80) main building (`+0xbb8`, `main`) for it. The Attack's own target in states 4 to 7
    /// or 11 is kept as it is (true). Else a unit without top speed (slot 0x208 equal to 0:
    /// `UCOMISS`/`JZ`, so a NaN speed asks on) or whose unit AI's canMoveToAttack (slot 0x1d0,
    /// `FUN_140c48420`, on the target's id: `can_move`) is false keeps it only within its reach:
    /// the footprint gap (`FUN_14118f680`) at most the range (`+0x90`), half a tile more for a
    /// range up to 0.1 (`COMISS`/`JA`: a NaN gap is within). Kept, the indirect fire flag
    /// (`+0xa0`, which genie's Attacks never set) clears, the target becomes the Attack's (slot
    /// 0x68, [`Attack::set_target`]) and where it was seen its place (`+0xa4`..`+0xac`); true.
    pub fn work(
        &mut self,
        target: Option<Mark>,
        main: Option<Mark>,
        unit: &Wielder,
        can_move: impl FnOnce(i32) -> bool,
    ) -> Worked {
        let Some(mut target) = target else {
            return Worked::default();
        };
        if 0.0 < unit.area_effect
            && target.kind == BUILDING
            && let Some(m) = main
        {
            target = m;
        }
        if target.id == self.target && (matches!(self.state, 4..=7) || self.state == 11) {
            return Worked {
                kept: true,
                ..Worked::default()
            };
        }
        if unit.speed == 0.0 || !can_move(target.id) {
            let gap = footprint_gap(unit.position, unit.radius, target.position, target.radius);
            if self.reach() < gap {
                return Worked::default();
            }
        }
        let mut worked = self.set_target(Some(target));
        self.seen = target.position;
        worked.kept = true;
        worked
    }

    /// Slot 0x68 (`FUN_140a82a10`, `set_target_obj`): a target of class 60 (`-0xe70` raw) changes
    /// nothing; another target than the Attack's clears the unit's attack count (`+0x900`); then
    /// the action's own (`FUN_140a7a230`): a new target lets the old one go (its slot 0x180, when
    /// the world still holds it: the caller's) unless the Attack is a sub-action (`+0x60`), and
    /// becomes the Attack's (`+0x20`, its id `+0x30`) below state 7, taken (its slot 0x178 with
    /// the action's type 9, the caller's) unless a sub-action; one in state 7 or more leaves the
    /// Attack without a target (−1), as does none.
    pub fn set_target(&mut self, target: Option<Mark>) -> Worked {
        let mut worked = Worked::default();
        if target.is_some_and(|t| t.class == CLASS_60) {
            return worked;
        }
        let new = target.map_or(-1, |t| t.id);
        if new != self.target {
            worked.clears_count = true;
        }
        if self.target != -1 && self.target != new && !self.sub_action {
            worked.lets_go = Some(self.target);
        }
        if self.target != new {
            self.target = match target {
                Some(t) if t.state < 7 => {
                    if !self.sub_action {
                        worked.takes = Some(t.id);
                    }
                    t.id
                }
                _ => -1,
            };
        }
        worked
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
        // a dead tree (class 15) up to state 5 keeps a non-villager's attack going: its update
        // goes on, the target not alive (`0x140a8342e`)
        let alive = s.victim.state <= 2;
        let gone =
            !alive && (s.victim.class != TREE || s.own.class == VILLAGER || 5 < s.victim.state);
        if self.state != KILLED && gone {
            // the target died before the attack ended (the update's start): one of stance 0 that
            // moves would wait (set_state 5), not modelled; a villager's unit AI keeps a dead
            // target of Gaia's (another's FUN_140c5e1e0 forgets, not modelled). The attack lets
            // the target go (slot 0x68 with none: a sub-action lets nothing go; the unit's attack
            // count clears, the caller's) and its point, posts no notify as the hunt's
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
            self.target = -1;
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
                match self.ready_to_attack(&mut walk.reborrow(), s, false)? {
                    // a target no longer alive (a dead tree): set_state(1), code 5
                    Readiness::Ready if !alive => {
                        self.finish(&mut walk, s)?;
                        return Ok(5);
                    }
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
                // the target goes (slot 0x68 with none: a sub-action lets nothing go; the unit's
                // attack count clears, the caller's) and the attack ends (set_state 1); the Hunt's
                // Attack, a sub-action, then posts 0x1fb with 600 and 0x202 too and returns 5
                if self.sub_action {
                    self.target = -1;
                    self.finish(&mut walk, s)?;
                    s.posted.push((0x1fb, [600, 0, 0x202]));
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
            // cases 4 and 10 walk a MoveTo to where the target was seen
            GOTO_SEEN | 11 => Err(Unsupported(
                "an Attack in state 5 or 11 (its MoveTo to where its target was seen)",
            )),
            // the states without a case (0, 3, 8 to 10, 12, 14 and above) return 0
            _ => Ok(0),
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
        match after_hit(
            ranged,
            *s.attack_timer,
            self.target != -1,
            self.arms.interrupt_frame,
            self.arms.fire_frame,
            sprite.frame,
        ) {
            AfterHit::Done => {
                self.enter(KILLED, walk, s)?;
                return Ok(0);
            }
            AfterHit::Ready => {
                self.ready(walk, s)?;
                return Ok(0);
            }
            // readyToAttack with its third argument 0: a target out of reach is followed
            // (`set_state` 4); a ready unit only faces it; the swing goes on either way
            AfterHit::Continue { check: true } => {
                if let Readiness::Away(gap) =
                    self.ready_to_attack(&mut walk.reborrow(), s, false)?
                {
                    self.follow(walk, s, gap)?;
                }
            }
            AfterHit::Continue { check: false } => {}
        }
        // the loop test reads the unit's sprite as it is now (`+0x140`): readyToAttack may have
        // shown the standing one
        let sprite = s.sprite.unwrap_or(sprite);
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
            if changed && let Some(current) = s.sprite.map(|sp| sp.graphic) {
                let _ = reset_sprite(s.sprite, current);
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
        let _ = damage::blast(&mut s.own, center, Some(target), None, &mut targets, 1.0)?;
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

/// What the swing does after its hit, before it waits for its sprite's loop (`FUN_140a82ae0`
/// case 6, `+0x9e` clear).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AfterHit {
    /// A ranged unit without a target (and no indirect fire, `+0xa0`, which the slice's attacks
    /// never have): `set_state(2)`.
    Done,
    /// A ranged unit past its interrupt frame, else its firing frame: `set_state(6)` at once.
    Ready,
    /// The swing goes on; at or past its interrupt frame readyToAttack (`FUN_140a7f9d0` with 0)
    /// runs first.
    Continue {
        /// readyToAttack runs.
        check: bool,
    },
}

/// [`AfterHit`] of a unit with a missile or not (`ranged`, `+0x98`), its attack timer (`+0x91c`,
/// slot 0x100: `COMISS` with 0 then `SETBE`, so NaN counts as run out), a target (`+0x20`), its
/// interrupt frame (`+0x9c`, −1 for none), its firing frame (`+0x9a`) and the sprite's frame
/// (the sprite list's slot 8: the node's `+0x24`), compared as signed shorts.
fn after_hit(
    ranged: bool,
    timer: f32,
    target: bool,
    interrupt: i16,
    fire: i16,
    frame: i16,
) -> AfterHit {
    if ranged && crate::fmath::at_most_or_unordered(timer, 0.0) {
        if !target {
            return AfterHit::Done;
        }
        let limit = if interrupt == -1 { fire } else { interrupt };
        if limit < frame {
            return AfterHit::Ready;
        }
    }
    AfterHit::Continue {
        check: interrupt != -1 && interrupt <= frame,
    }
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
        let _ = damage::blast(own, hit.at, None, Some(main), &mut targets, 1.0)?;
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
    look: Option<&crate::abilities::Look>,
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

/// readyToAttack's ready-at-once range test of a unit that does not move (`0x140a7fd92`): the
/// gap `near` (half a tile less for a range up to 0.1) is not beyond the range (`COMISS`/`JA`), and
/// the least range is at most 0.1 or at most the gap (`COMISS`/`JBE` each: NaN passes).
fn in_range(near: f32, range: f32, min_range: f32) -> bool {
    use crate::fmath::at_most_or_unordered;
    at_most_or_unordered(near, range)
        && (at_most_or_unordered(min_range, NO_RANGE) || at_most_or_unordered(min_range, near))
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

    /// `FUN_140a82ae0` case 6 after the hit: a ranged unit whose attack timer ran out is done
    /// without a target and ready again past its interrupt frame (its firing frame without one);
    /// at or past the interrupt frame readyToAttack runs, the swing going on.
    #[test]
    fn a_swing_after_its_hit_follows_its_interrupt_and_firing_frames() {
        use AfterHit::{Continue, Done, Ready};
        // ranged, timer run out (NaN counts too)
        assert_eq!(after_hit(true, 0.0, false, 5, 3, 9), Done);
        assert_eq!(after_hit(true, 0.0, true, 5, 3, 6), Ready);
        assert_eq!(after_hit(true, f32::NAN, true, 5, 3, 6), Ready);
        assert_eq!(after_hit(true, -1.0, true, -1, 3, 4), Ready);
        assert_eq!(
            after_hit(true, 0.0, true, -1, 3, 3),
            Continue { check: false }
        );
        assert_eq!(
            after_hit(true, 0.0, true, 5, 3, 5),
            Continue { check: true }
        );
        // ranged with time left, or melee: only the interrupt test
        assert_eq!(
            after_hit(true, 0.5, false, 5, 3, 9),
            Continue { check: true }
        );
        assert_eq!(
            after_hit(false, 0.0, false, 5, 3, 4),
            Continue { check: false }
        );
        assert_eq!(
            after_hit(false, 0.0, true, 5, 3, 5),
            Continue { check: true }
        );
        assert_eq!(
            after_hit(false, 0.0, true, -1, 3, 9),
            Continue { check: false }
        );
    }

    /// A unit's own Attack on `target` in `state` with range `range`.
    fn own_attack(target: i32, state: u8, range: f32) -> Attack {
        Attack {
            target,
            target2: -1,
            state,
            timer: 0.0,
            range,
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
                idle_sprite: None,
                idle_attack_sprite: None,
                missile: -1,
                fire_frame: 0,
                interrupt_frame: -1,
            },
            sub_action: false,
            repaths: false,
            runs: true,
        }
    }

    /// Slot 0x38 (`FUN_140a82ac0`): `set_state(1)`: its `MoveTo` goes, stopping the unit and its
    /// animation, then the animation runs at speed 1 and the unit stands; the target stays.
    #[test]
    fn a_stop_ends_the_attack_and_the_unit_stands() {
        let stand = Graphic {
            id: 1,
            frames: 10,
            facets: 8,
            speed_multiplier: 1.0,
            frame_duration: 0.1,
            replay_delay: 0.0,
            flags: 0,
        };
        let mut a = own_attack(77, MOVE, 1.0);
        a.move_to = Some(MoveTo::Moving);
        a.arms.idle_sprite = Some(stand);
        let mut mover = crate::hunt::tests::walker();
        let (mut speed, mut sprite, mut rng) = (Some(0.5), None, Well512::default());
        a.halt(Some(&mut mover), &mut speed, &mut sprite, &mut rng)
            .unwrap();
        assert_eq!(
            (a.state, a.move_to, a.target, speed),
            (DONE, None, 77, Some(1.0))
        );
        assert_eq!(sprite.map(|s| s.graphic.id), Some(1));
        assert_eq!(mover.velocity.unwrap().velocity[2], 0.0);
        // a unit without a standing sprite is not modelled
        let mut bare = own_attack(77, READY, 1.0);
        assert!(bare.halt(None, &mut speed, &mut sprite, &mut rng).is_err());
    }

    /// An object of class 9 in state 2 at `x` on the line y = 10, radii 0.2.
    fn mark(id: i32, x: f32) -> Mark {
        Mark {
            id,
            kind: 70,
            class: 9,
            state: 2,
            position: [x, 10.0, 0.5],
            radius: [0.2, 0.2],
        }
    }

    /// A lion (speed 1.2, radii 0.4) at (10, 10) without an area effect.
    const LION: Wielder = Wielder {
        area_effect: 0.0,
        speed: 1.2,
        position: [10.0, 10.0, 0.0],
        radius: [0.4, 0.4],
    };

    /// `FUN_140a841d0`: the Attack's own target in states 4 to 7 or 11 is kept as it is; in
    /// another state a unit that may move takes it again: where it was seen becomes its place,
    /// and the same target changes nothing else.
    #[test]
    fn an_attack_keeps_its_own_target_and_sees_it_again_out_of_its_moving_states() {
        let deer = mark(7, 13.0);
        for state in [4, 5, 6, 7, 11] {
            let mut a = own_attack(7, state, 0.0);
            let w = a.work(Some(deer), None, &LION, |_| panic!("no range test"));
            assert_eq!(
                w,
                Worked {
                    kept: true,
                    ..Worked::default()
                }
            );
            assert_eq!(a.seen, [NONE; 3], "{state}");
        }
        for state in [1, 2, 3, 13] {
            let mut a = own_attack(7, state, 0.0);
            let w = a.work(Some(deer), None, &LION, |id| id == 7);
            assert_eq!(
                w,
                Worked {
                    kept: true,
                    ..Worked::default()
                }
            );
            assert_eq!((a.target, a.seen), (7, deer.position), "{state}");
        }
        // none answers false
        let mut a = own_attack(7, 1, 0.0);
        assert_eq!(a.work(None, None, &LION, |_| true), Worked::default());
    }

    /// The range test of slot 9: a unit without speed, or whose unit AI may not move to attack,
    /// keeps a target only within its reach (the range, half a tile more up to 0.1; the gap
    /// between the footprints); a NaN speed asks the unit AI, a NaN gap is within (`JA`).
    #[test]
    fn an_attack_that_cannot_follow_keeps_a_target_only_in_reach() {
        // the gap: 13 - 10 - (0.4 + 0.2) = 2.4
        let deer = mark(7, 13.0);
        let reach = |range: f32, speed: f32, can: bool| {
            let mut a = own_attack(7, 1, range);
            let unit = Wielder { speed, ..LION };
            a.work(Some(deer), None, &unit, |_| can).kept
        };
        assert!(reach(0.0, 1.2, true));
        assert!(!reach(2.0, 1.2, false));
        assert!(!reach(2.0, 0.0, true));
        assert!(reach(2.4, 0.0, true));
        assert!(reach(2.5, 0.0, false));
        // a range up to 0.1 reaches half a tile more: 0.1 + 0.5 against a gap of 0.6
        let near = mark(7, 11.2);
        let mut a = own_attack(7, 1, 0.1);
        assert!(
            a.work(Some(near), None, &Wielder { speed: 0.0, ..LION }, |_| true)
                .kept
        );
        let mut a = own_attack(7, 1, 0.0);
        let farther = mark(7, 11.3);
        assert!(
            !a.work(Some(farther), None, &Wielder { speed: 0.0, ..LION }, |_| {
                true
            })
            .kept
        );
        // UCOMISS/JP: a NaN speed is not 0, so the unit AI decides
        let mut asked = false;
        let mut a = own_attack(7, 1, 0.0);
        let unit = Wielder {
            speed: f32::NAN,
            ..LION
        };
        assert!(
            a.work(Some(deer), None, &unit, |_| {
                asked = true;
                true
            })
            .kept
        );
        assert!(asked);
        // COMISS/JA: a NaN gap does not lie beyond the reach
        let lost = Mark {
            position: [f32::NAN, 10.0, 0.0],
            ..deer
        };
        let mut a = own_attack(7, 1, 0.0);
        assert!(
            a.work(Some(lost), None, &Wielder { speed: 0.0, ..LION }, |_| false)
                .kept
        );
    }

    /// An area effect's Attack takes a building's main building (`+0xbb8`): another target than
    /// its own clears the unit's attack count, lets the old one go and takes the new one (an own
    /// Attack; a sub-action does neither); a target in state 7 or more is not taken, and one of
    /// class 60 changes nothing but where it was seen.
    #[test]
    fn an_area_effect_s_attack_takes_a_building_s_main_building() {
        let annex = Mark {
            kind: BUILDING,
            class: 3,
            ..mark(30, 11.0)
        };
        let main = Mark {
            id: 31,
            position: [12.0, 10.0, 0.0],
            radius: [1.5, 1.5],
            ..annex
        };
        let mangonel = Wielder {
            area_effect: 1.0,
            ..LION
        };
        let mut a = own_attack(30, 6, 0.0);
        let w = a.work(Some(annex), Some(main), &mangonel, |id| id == 31);
        assert_eq!(
            w,
            Worked {
                kept: true,
                clears_count: true,
                lets_go: Some(30),
                takes: Some(31),
            }
        );
        assert_eq!((a.target, a.seen), (31, main.position));
        // without an area effect the annex itself is the target, its own in state 6
        let mut a = own_attack(30, 6, 0.0);
        assert_eq!(
            a.work(Some(annex), Some(main), &LION, |_| true),
            Worked {
                kept: true,
                ..Worked::default()
            }
        );
        // a sub-action lets go and takes nothing
        let mut a = own_attack(30, 6, 0.0);
        a.sub_action = true;
        let w = a.work(Some(annex), Some(main), &mangonel, |_| true);
        assert_eq!(
            (w.lets_go, w.takes, w.clears_count, a.target),
            (None, None, true, 31)
        );
        // a new target in state 7 or more leaves the Attack without one
        let mut a = own_attack(30, 6, 0.0);
        let dying = Mark { state: 7, ..main };
        let w = a.work(Some(annex), Some(dying), &mangonel, |_| true);
        assert_eq!((w.lets_go, w.takes, a.target), (Some(30), None, -1));
        // class 60: slot 0x68 returns at once; the seen point still follows
        let mut a = own_attack(7, 1, 0.0);
        let odd = Mark {
            class: CLASS_60,
            ..mark(8, 10.5)
        };
        let w = a.work(Some(odd), None, &LION, |_| true);
        assert_eq!(
            (w, a.target, a.seen),
            (
                Worked {
                    kept: true,
                    ..Worked::default()
                },
                7,
                odd.position
            )
        );
        // slot 0x68 with none: the count clears and the old target goes
        let mut a = own_attack(7, 1, 0.0);
        assert_eq!(
            a.set_target(None),
            Worked {
                clears_count: true,
                lets_go: Some(7),
                ..Worked::default()
            }
        );
        assert_eq!(a.target, -1);
    }

    /// `FUN_1413b9f00`: the sprite of the graphic asked for goes back to its first frame (true);
    /// another graphic's, or none, is left (false).
    #[test]
    fn a_sprite_resets_only_for_its_own_graphic() {
        let swing = Graphic {
            id: 224,
            frames: 10,
            facets: 8,
            speed_multiplier: 1.0,
            frame_duration: 0.1,
            replay_delay: 0.0,
            flags: 1,
        };
        let other = Graphic { id: 225, ..swing };
        let mut sprite = Some(ActiveSprite {
            frame: 6,
            last_frame: 5,
            looped: true,
            ..ActiveSprite::start(swing, &mut Well512::default())
        });
        let before = sprite;
        assert!(!reset_sprite(&mut sprite, other));
        assert_eq!(sprite, before);
        assert!(reset_sprite(&mut sprite, swing));
        let s = sprite.unwrap();
        assert_eq!((s.frame, s.last_frame, s.looped), (0, 0, false));
        assert!(!reset_sprite(&mut None, swing));
    }

    /// A soldier (class 6, type 70, speed 0.8, radius 0.2) as a mover.
    fn soldier_mover(speed: f32) -> Mover {
        use crate::movement::{MoveStats, Velocity};
        Mover {
            forward: [1.0, 0.0, 0.0],
            right: [0.0, 0.0, 1.0],
            velocity: Some(Velocity {
                velocity: [0.0, 0.0, 0.0],
                acceleration: [0.0, 0.0, 0.0],
            }),
            paths: vec![],
            factor: 1.0,
            stats: MoveStats {
                fly_mode: 0,
                speed,
                turn_rate: f32::MAX,
                turn_radius: 0.0,
                turn_radius_speed: f32::MAX,
                radius: [0.2, 0.2, 0.5],
                restriction: 7,
                restriction_mask: 6,
                turn_time: 0.0,
                move_algorithm: 0,
                min_collision: 1.0,
                fast_turn: false,
                walk_speed: speed,
                stand_speed: speed,
                run_speed: speed,
                facets: Some(crate::facing::SpriteFacets {
                    count: 8,
                    flag_1: true,
                }),
                weapon_range: 0.0,
            },
            waiting: 0,
            wait_delays: 0,
            attack_moving: false,
            last_move: 0,
            moved_at: [0.0; 3],
            substitute_count: 0,
            ignore_units: Vec::new(),
            user_waypoints: Vec::new(),
            last_turn: 0.0,
            angle: 0.0,
            move_to_point: None,
            speed_stopped: false,
            pauses: Vec::new(),
            in_the_way: Vec::new(),
            sprite_facets: None,
        }
    }

    const STANDING: Graphic = Graphic {
        id: 300,
        frames: 10,
        facets: 8,
        speed_multiplier: 1.0,
        frame_duration: 0.1,
        replay_delay: 0.0,
        flags: 1,
    };

    /// What an Attack's update reads besides its walk: the unit's sprite, timers and target.
    struct Rig {
        sprite: Option<ActiveSprite>,
        anim: Option<f32>,
        timer: f32,
        reload_worldtime: u32,
        victim: Victim,
        blasted: Vec<Victim>,
        rng: Well512,
    }

    impl Rig {
        fn new(victim: Victim) -> Self {
            Self {
                sprite: None,
                anim: Some(0.0),
                timer: 0.0,
                reload_worldtime: 0,
                victim,
                blasted: Vec::new(),
                rng: Well512::default(),
            }
        }

        fn strike(&mut self) -> Strike<'_> {
            Strike {
                sprite: &mut self.sprite,
                anim_speed: &mut self.anim,
                standing: Some(STANDING),
                volleys: false,
                attack_timer: &mut self.timer,
                reload_worldtime: &mut self.reload_worldtime,
                reload: 2.0,
                hit_chance: 100,
                victim: &mut self.victim,
                blasted: &mut self.blasted,
                sprite_rng: &mut self.rng,
                own: Victim {
                    id: 1,
                    kind: 70,
                    class: 6,
                    player: 1,
                    ..Victim::default()
                },
                executed: None,
                fire: None,
                can_move: true,
                stance: 0,
                posted: Vec::new(),
                queue_empty: true,
                world_time: 0,
                ai_order: Some(-1),
                ai_type: None,
                computer: false,
                armed: true,
                volley_under_way: false,
                let_go: None,
                forgets: false,
                sends_home: false,
            }
        }
    }

    /// A world holding the target `id` of `player` and class `class` at `at`, radius 0.2.
    fn world_with(id: i32, player: usize, class: i16, at: [f32; 3]) -> crate::spatial::Spatial {
        let mut world = crate::spatial::Spatial::default();
        world.insert(
            id,
            crate::spatial::Placed {
                position: at,
                radius: [0.2, 0.2],
                class,
                player,
                master: 4,
                obstruction_type: 0,
                buildable_on: 0,
                tall: false,
                speed: 0.0,
                unknown: None,
                obstruction: None,
            },
        );
        world
    }

    fn walk_in<'a>(mover: &'a mut Mover, world: &'a mut crate::spatial::Spatial) -> Walk<'a> {
        Walk {
            mover: Some(mover),
            me: crate::movement::Me {
                id: 1,
                position: [10.0, 10.0, 0.0],
                action_state: READY,
                group: -1,
            },
            dt: 0.02,
            world,
            plan: None,
        }
    }

    /// An own Attack of the soldier at (10, 10) on `target` in state 6, range `range` and least
    /// range `min`, its point `point`.
    fn soldier_attack(target: i32, range: f32, min: f32, point: [f32; 3]) -> Attack {
        let mut a = own_attack(target, READY, range);
        a.min_range = min;
        a.point = point;
        a.arms.idle_sprite = Some(STANDING);
        a
    }

    /// readyToAttack below the least range: a unit's own Attack posts 0x1fe (−1, −1, −1); a unit
    /// that may move waits in its state, one without speed fails (`set_state` 13: the standing
    /// sprite at speed 1).
    #[test]
    fn ready_to_attack_under_the_least_range_posts_0x1fe_and_waits_or_fails() {
        let at = [11.0, 10.0, 0.0];
        let target = || Victim {
            id: 7,
            position: at,
            ..Victim::default()
        };
        for (speed, state) in [(0.8, READY), (0.0, FAILED)] {
            let mut world = world_with(7, 0, 6, at);
            let mut mover = soldier_mover(speed);
            let mut walk = walk_in(&mut mover, &mut world);
            let mut rig = Rig::new(target());
            let mut s = rig.strike();
            let mut a = soldier_attack(7, 4.0, 2.0, [NONE; 3]);
            let r = a.ready_to_attack(&mut walk, &mut s, false).unwrap();
            assert!(matches!(r, Readiness::Waiting));
            assert_eq!((a.state, s.posted.clone()), (state, vec![(0x1fe, [-1; 3])]));
            if state == FAILED {
                assert_eq!(*s.anim_speed, Some(1.0));
                assert_eq!(s.sprite.map(|sp| sp.graphic.id), Some(STANDING.id));
            }
        }
        // a sub-action posts nothing
        let mut world = world_with(7, 0, 6, at);
        let mut mover = soldier_mover(0.8);
        let mut walk = walk_in(&mut mover, &mut world);
        let mut rig = Rig::new(target());
        let mut s = rig.strike();
        let mut a = soldier_attack(7, 4.0, 2.0, [NONE; 3]);
        a.sub_action = true;
        let _ = a.ready_to_attack(&mut walk, &mut s, false).unwrap();
        assert_eq!(s.posted, Vec::new());
    }

    /// readyToAttack beyond the reach: a unit's own Attack posts 0x1fd; a unit that may move
    /// follows (`set_state` 4 is the caller's), one that cannot fails (`set_state` 13).
    #[test]
    fn ready_to_attack_out_of_reach_follows_or_fails() {
        let at = [15.0, 10.0, 0.0];
        for (speed, can_move, follows) in
            [(0.8, true, true), (0.0, true, false), (0.8, false, false)]
        {
            let mut world = world_with(7, 0, 6, at);
            let mut mover = soldier_mover(speed);
            let mut walk = walk_in(&mut mover, &mut world);
            let mut rig = Rig::new(Victim {
                id: 7,
                position: at,
                ..Victim::default()
            });
            let mut s = rig.strike();
            s.can_move = can_move;
            let mut a = soldier_attack(7, 1.0, 0.0, [NONE; 3]);
            let r = a.ready_to_attack(&mut walk, &mut s, false).unwrap();
            assert_eq!(s.posted, vec![(0x1fd, [0; 3])]);
            assert_eq!(matches!(r, Readiness::Away(_)), follows);
            assert_eq!(a.state, if follows { READY } else { FAILED });
        }
    }

    /// readyToAttack on a player's target out of sight: a unit of stance 0 with speed that may
    /// move would wait (`set_state` 5, not modelled); any other fails (`set_state` 13).
    #[test]
    fn ready_to_attack_on_a_target_out_of_sight_fails_or_would_wait() {
        let at = [11.0, 10.0, 0.0];
        for (stance, fails) in [(0, false), (2, true)] {
            let mut world = world_with(7, 2, 6, at);
            world.sight = Some(crate::visibility::Visibility::new(
                16,
                16,
                Vec::new(),
                vec![0; 256],
                Vec::new(),
                0,
                Vec::new(),
            ));
            let mut mover = soldier_mover(0.8);
            let mut walk = walk_in(&mut mover, &mut world);
            let mut rig = Rig::new(Victim {
                id: 7,
                player: 2,
                position: at,
                ..Victim::default()
            });
            let mut s = rig.strike();
            s.stance = stance;
            let mut a = soldier_attack(7, 4.0, 0.0, at);
            let r = a.ready_to_attack(&mut walk, &mut s, false);
            if fails {
                assert!(matches!(r, Ok(Readiness::Waiting)));
                assert_eq!(a.state, FAILED);
            } else {
                assert!(r.is_err());
            }
        }
    }

    /// readyToAttack's ready-at-once test: a target on the attack's point is ready at once, but
    /// not with the third argument; a NaN difference from the point is not within half a tile
    /// (the reach decides and the target's place becomes the point); a NaN attack timer has run
    /// out (slot 0x100's `SETBE`).
    #[test]
    fn ready_to_attack_s_at_once_test_and_its_nan_paths() {
        let at = [11.0, 10.0, 0.0];
        let run = |point: [f32; 3], skip: bool, range: f32, timer: f32| {
            let mut world = world_with(7, 0, 6, at);
            let mut mover = soldier_mover(0.8);
            let mut walk = walk_in(&mut mover, &mut world);
            let mut rig = Rig::new(Victim {
                id: 7,
                position: at,
                ..Victim::default()
            });
            rig.timer = timer;
            let mut s = rig.strike();
            let mut a = soldier_attack(7, range, 0.0, point);
            let r = a.ready_to_attack(&mut walk, &mut s, skip).unwrap();
            (r, a.point)
        };
        assert!(matches!(run(at, false, 0.0, 0.0).0, Readiness::Ready));
        // the third argument skips it: the gap 0.6 lies beyond a reach of 0.5
        assert!(matches!(run(at, true, 0.0, 0.0).0, Readiness::Away(_)));
        // a NaN point: moved and far, the reach decides (2 > 0.6) and the point follows
        let (r, point) = run([f32::NAN, 10.0, 0.0], false, 2.0, 0.0);
        assert!(matches!(r, Readiness::Ready));
        assert_eq!(point, at);
        assert!(matches!(run(at, false, 0.0, f32::NAN).0, Readiness::Ready));
        assert!(matches!(run(at, false, 0.0, 0.5).0, Readiness::Waiting));
    }

    /// The ready-at-once range test of a unit that does not move: `COMISS`/`JA` past the range,
    /// `JBE` on each least-range test, so NaN passes all three.
    #[test]
    fn the_ready_at_once_range_test_passes_nan() {
        assert!(in_range(1.0, 2.0, 0.0));
        assert!(!in_range(3.0, 2.0, 0.0));
        assert!(in_range(f32::NAN, 2.0, 0.0));
        assert!(in_range(1.0, f32::NAN, 0.0));
        // a least range above 0.1 needs the gap at least as far
        assert!(!in_range(0.5, 2.0, 1.0));
        assert!(in_range(1.5, 2.0, 1.0));
        assert!(in_range(0.5, 2.0, f32::NAN));
        assert!(in_range(f32::NAN, 2.0, 1.0));
    }

    /// `FUN_140a82ae0`'s states without a case (0, 3, 8 to 10, 12, 14) return 0; their timer
    /// runs.
    #[test]
    fn an_attack_in_a_state_without_a_case_returns_0() {
        let at = [11.0, 10.0, 0.0];
        for state in [0, 3, 8, 9, 10, 12, 14] {
            let mut world = world_with(7, 0, 6, at);
            let mut mover = soldier_mover(0.8);
            let walk = walk_in(&mut mover, &mut world);
            let mut rig = Rig::new(Victim {
                id: 7,
                state: 2,
                position: at,
                ..Victim::default()
            });
            let mut s = rig.strike();
            let mut a = soldier_attack(7, 1.0, 0.0, at);
            (a.state, a.seen) = (state, at);
            assert_eq!(a.update(walk, &mut s), Ok(0), "{state}");
            assert_eq!((a.state, a.timer), (state, 0.02));
        }
    }

    /// Case 1 (state 2) once the swing's sprite is done: the Hunt's Attack, a sub-action, lets
    /// its target go (slot 0x68 with none), ends (`set_state` 1) and then posts 0x1fb with 600 and
    /// 0x202 before it returns 5.
    #[test]
    fn a_sub_action_s_attack_after_its_kill_posts_0x1fb_and_returns_5() {
        let at = [11.0, 10.0, 0.0];
        let mut world = world_with(7, 0, 9, at);
        let mut mover = soldier_mover(0.8);
        let walk = walk_in(&mut mover, &mut world);
        let mut rig = Rig::new(Victim {
            id: 7,
            state: 3,
            position: at,
            ..Victim::default()
        });
        let mut s = rig.strike();
        let mut a = soldier_attack(7, 0.0, 0.0, at);
        (a.state, a.seen, a.sub_action) = (KILLED, at, true);
        assert_eq!(a.update(walk, &mut s), Ok(5));
        assert_eq!((a.state, a.target), (DONE, -1));
        assert_eq!(s.posted, vec![(0x1fb, [600, 0, 0x202])]);
        assert_eq!(*s.anim_speed, Some(1.0));
        assert_eq!(s.let_go, None);
    }

    /// A dead tree (class 15) up to state 5 keeps a non-villager's attack going; ready, the
    /// attack on it ends (`set_state` 1) and returns 5. A villager's, or one on a tree past
    /// state 5, ends at the update's start, letting the target go.
    #[test]
    fn a_dead_tree_keeps_a_soldier_s_attack_until_it_is_ready() {
        let at = [11.0, 10.0, 0.0];
        let run = |unit_class: i16, tree_state: u8| {
            let mut world = world_with(7, 0, TREE, at);
            let mut mover = soldier_mover(0.8);
            let walk = walk_in(&mut mover, &mut world);
            let mut rig = Rig::new(Victim {
                id: 7,
                class: TREE,
                state: tree_state,
                position: at,
                ..Victim::default()
            });
            let mut s = rig.strike();
            s.own.class = unit_class;
            let mut a = soldier_attack(7, 1.0, 0.0, at);
            a.seen = at;
            let code = a.update(walk, &mut s).unwrap();
            (code, a.state, s.let_go)
        };
        assert_eq!(run(6, 3), (5, DONE, None));
        assert_eq!(run(6, 5), (5, DONE, None));
        assert_eq!(run(4, 3), (5, DONE, Some(7)));
        assert_eq!(run(6, 6), (5, DONE, Some(7)));
    }

    /// After a hit, the swing's loop test reads the unit's sprite as readyToAttack left it: one
    /// that failed (`set_state` 13, the standing sprite from its first frame) keeps the attack in
    /// state 13, though the attack sprite had looped.
    #[test]
    fn a_swing_that_fails_its_interrupt_check_stays_failed() {
        let at = [15.0, 10.0, 0.0];
        let mut world = world_with(7, 0, 6, at);
        let mut mover = soldier_mover(0.0);
        let walk = walk_in(&mut mover, &mut world);
        let mut rig = Rig::new(Victim {
            id: 7,
            state: 2,
            position: at,
            ..Victim::default()
        });
        let swing = Graphic {
            id: 301,
            ..STANDING
        };
        rig.sprite = Some(ActiveSprite {
            looped: true,
            frame: 9,
            ..ActiveSprite::start(swing, &mut Well512::default())
        });
        let mut s = rig.strike();
        let mut a = soldier_attack(7, 1.0, 0.0, [NONE; 3]);
        (a.state, a.seen, a.arms.interrupt_frame) = (SWING, at, 5);
        assert_eq!(a.update(walk, &mut s), Ok(0));
        assert_eq!(a.state, FAILED);
        assert_eq!(s.posted, vec![(0x1fd, [0; 3])]);
        assert_eq!(s.sprite.map(|sp| sp.graphic.id), Some(STANDING.id));
    }

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

    /// `FUN_140a7c210`: without a `MoveTo` sprite of its own the Attack walks with the unit's
    /// running sprite, else its walking one, slots 0x7d0 and 0x7c8: a terrain look's while the
    /// unit has one; the attack sprite stays the master's.
    #[test]
    fn an_attack_walks_with_the_unit_s_running_or_walking_slot() {
        let g = |id| Graphic {
            id,
            frames: 10,
            facets: 8,
            speed_multiplier: 1.0,
            frame_duration: 0.1,
            replay_delay: 0.0,
            flags: 1,
        };
        let graphics: std::collections::BTreeMap<i16, Graphic> = [10, 11, 12, 20, 21]
            .into_iter()
            .map(|id| (id, g(id)))
            .collect();
        let master = crate::masters::Master {
            move_sprite: 10,
            run_sprite: 11,
            attack_sprite: 12,
            ..crate::masters::Master::default()
        };
        let a = arms(&master, &graphics, [-1; 4], -1);
        assert_eq!(a.move_sprite.map(|s| s.id), Some(11));
        let look = crate::abilities::Look {
            standing: None,
            walking: Some(20),
            attacking: Some(21),
            running: None,
            dying: None,
            undead: None,
        };
        let a = arms_looking(&master, Some(&look), &graphics, [-1; 4], -1);
        assert_eq!(a.move_sprite.map(|s| s.id), Some(20));
        assert_eq!(a.sprite.map(|s| s.id), Some(12));
        // its own MoveTo sprite first
        let a = arms_looking(&master, Some(&look), &graphics, [-1; 4], 10);
        assert_eq!(a.move_sprite.map(|s| s.id), Some(10));
    }
}
