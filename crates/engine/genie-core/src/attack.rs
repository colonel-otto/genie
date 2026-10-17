//! The Attack action (`RGE_Action_Attack`, type 9, vtable `0x142ed47a8`) up to its walk: a
//! hunting villager's sub-action on a tree ([`crate::hunt`]). Built away from its target, it walks
//! there with a MoveTo sub-action ([`crate::move_to`]); its attack (states 6 and 7) is not
//! modelled yet.
//!
//! 158041: constructor `FUN_140a7c210`, start (slot 5) `0x140a7f8c0`, update (slot 6)
//! `0x140a82ae0`, set_state (slot 16) `0x140a820c0`; the range test `FUN_140a7f9d0`
//! (`readyToAttack`).

use crate::move_to::{Goal, MoveTo, Walk, start_walk, walk_on};
use crate::movement::{Mover, footprint_gap_squared};
use crate::unit_ai::Unsupported;

/// The Attack action's type, which word 9 adds.
pub const ATTACK: i16 = 9;

/// A range at most this counts as none (`DAT_142eb5690`): the range test widens the gap by
/// `-0.5` (`DAT_142eb6e58`) and the reach by `0.5` (`DAT_142e7f384`).
const NO_RANGE: f32 = 0.1;
const HALF: f32 = 0.5;
/// The squared reach of an attack without a range (`DAT_142ed48e4`).
const TOUCH: f32 = 0.01;
/// A MoveTo starts arrived within this gap (`DAT_142eb99cc`).
const ARRIVED: f32 = 0.01;
/// The gap below which the game takes a distance as zero (`DAT_142eb0c60`).
const EPSILON: f32 = 1e-7;
/// The point an attack on an object has none of (`DAT_14330ec20`).
const NONE: f32 = -1.0;

/// The states the slice models: 4, on the way to the target.
pub const MOVE: u8 = 4;

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

impl Attack {
    /// The constructor (`FUN_140a7c210`, from Hunt's set_state 12 with the master's weapon range,
    /// least range, missile and frame) and the start (slot 5 with 1, `0x140a7f8c0`): the target is
    /// the object itself (the area-effect and gate cases are not reached by a villager); a range of
    /// 0 becomes slot 0x238's. The start runs the range test; a unit out of reach that may move
    /// turns to state 4 there and again in the start, each time with a new MoveTo (whose teardown
    /// stops the unit). The unit's `+0x900` (attack count) is cleared and its sprites set; the
    /// slice holds neither.
    pub fn start(target: i32, e: &Engagement, mover: &mut Mover) -> Result<Self, Unsupported> {
        let mut a = Attack {
            target,
            state: 0,
            timer: 0.0,
            range: e.range,
            min_range: e.min_range,
            point: [NONE; 3],
            seen: e.target_position,
            move_to: None,
        };
        // readyToAttack (`FUN_140a7f9d0`): the target has not moved from the attack's point, which
        // is none, so only the reach decides
        if a.min_range > e.gap {
            return Err(Unsupported(
                "an attack on a target closer than the least range",
            ));
        }
        let mut reach = a.range;
        if reach <= NO_RANGE {
            reach += HALF;
        }
        if reach >= e.gap {
            return Err(Unsupported(
                "an attack whose target is in reach when it starts",
            ));
        }
        if e.stance == 1 {
            return Err(Unsupported("an attack's leash test (stance 1)"));
        }
        if e.speed == 0.0 || !e.can_move {
            return Err(Unsupported(
                "an attack whose unit cannot move to its target",
            ));
        }
        a.set_move(e, mover)?;
        // the start's own test of the speed and the unit AI: state 4 again
        a.set_move(e, mover)?;
        Ok(a)
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

    /// The teardown of the sub-actions (`FUN_140a73fc0` on the action list): a MoveTo stops the unit.
    pub fn tear_down(&mut self, mover: Option<&mut Mover>) -> Result<(), Unsupported> {
        if self.move_to.take().is_some()
            && let Some(m) = mover
        {
            m.stop()?;
        }
        Ok(())
    }

    /// One update (slot 6, `0x140a82ae0`) in state 4: the timer runs, the target is still there
    /// and where it was seen, the MoveTo walks (`FUN_140a992b0`); a unit still farther than its
    /// reach goes on (code 0). Returns the action list's code. The unit's sprites (`+0x68`,
    /// `+0x70`, `+0x80`) are not held.
    pub fn update(&mut self, mut walk: Walk<'_>) -> Result<u8, Unsupported> {
        self.timer += walk.dt;
        if self.state != MOVE {
            return Err(Unsupported("an Attack in a state other than 4"));
        }
        let t = walk
            .world
            .get_known(self.target)?
            .ok_or(Unsupported("an Attack whose target is gone"))?
            .clone();
        if t.position != self.seen {
            return Err(Unsupported("an Attack whose target moved"));
        }
        let code = match self.move_to {
            Some(MoveTo::Start) => {
                self.move_to = Some(MoveTo::Moving);
                let goal = Goal::Target(self.target);
                self.move_to = Some(start_walk(walk.reborrow(), goal, self.range)?);
                0
            }
            Some(MoveTo::Moving) => {
                self.move_to = Some(walk_on(walk.reborrow())?);
                0
            }
            Some(MoveTo::Arrived) => return Err(Unsupported("an Attack whose MoveTo arrived")),
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
        let reach = if NO_RANGE < self.range {
            self.range * self.range
        } else {
            TOUCH
        };
        if reach.abs() <= EPSILON || reach < footprint_gap_squared(at, ground, t.position, t.radius)
        {
            return Ok(code);
        }
        Err(Unsupported(
            "an Attack in reach of its target (set_state 6)",
        ))
    }
}

/// The gap between two footprints, for an [`Engagement`].
pub fn gaps(a: [f32; 3], a_radius: [f32; 2], b: [f32; 3], b_radius: [f32; 2]) -> (f32, f32) {
    let squared = footprint_gap_squared(a, a_radius, b, b_radius);
    (
        squared,
        crate::movement::footprint_gap(a, a_radius, b, b_radius),
    )
}
