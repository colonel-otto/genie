//! A group's update (FD-036): `BUnitGroup::update` (`FUN_14103d910`), which the world runs for
//! the groups a player owns before the player's objects, and the deletion of a group whose update
//! returns 0 (the world list's unlink, then `BUnitGroup`'s destructor, `FUN_140fe1920`).

use crate::GenieError;
use crate::group::UnitGroup;

mod awake;
mod checks;
mod entry;
mod moves;

pub use self::awake::IDLE_SPAN;

/// The group's current command (`mCurrentCommand`, `+0x3a8`) without one.
const NO_COMMAND: i32 = -1;
/// The group commands the update tells apart: an attack, a build, a repair, a relic's pick-up
/// and its delivery.
const ATTACK: i32 = 9;
const BUILD: i32 = 0x65;
const REPAIR: i32 = 0x6a;
const PICK_UP_RELIC: i32 = 0x84;
const DELIVER_RELIC: i32 = 0x88;
/// The action types of the tasks that pick up and deliver relics (`FUN_1415c60b0`'s third
/// argument, as the commands).
const RELIC_PICKUP_ACTION: i16 = 0x84;
const RELIC_DELIVERY_ACTION: i16 = 0x88;
/// The relics' master class (`FUN_141017c70`'s test of the target's class).
const RELIC_CLASS: i16 = 0x2a;
/// The monastery's master id (`FUN_141017850`'s test of the target's master).
const MONASTERY: i16 = 104;
/// The unit AI's order on a target (`+0x3c`) and its action on a relic (`+0x44`) that
/// `FUN_141017c70` and `FUN_141017850` read.
const TARGET_ORDER: i32 = crate::unit_ai::ORDER_ON_TARGET;
const RELIC_ACTION: i32 = 0x277;
/// The unit AI's action that defers to its order (`FUN_141017f00`).
const CHOSEN_ACTION: i32 = 0x25b;
/// The unit AI's attack action (`FUN_141007ca0`'s test, 600).
const ATTACK_ACTION: i32 = 0x258;
/// The master types from which an object moves (`RGE_Moving_Object`), acts
/// (`RGE_Action_Object`) and is a TRIBE combat unit.
const MOVING_KIND: u8 = 30;
const ACTION_KIND: u8 = 40;
const COMBAT_KIND: u8 = 70;
/// The trade cart's and the trade boat's master classes (`FUN_141017f00`).
const TRADE_CART: i16 = 19;
const TRADE_BOAT: i16 = 2;
/// The nearest member's target starts this far (`FUN_141007ca0`, `DAT_14330ebf4`).
const FARTHEST: f32 = crate::image::group::FARTHEST;
/// A follow (`mCurrentCommand` 2).
const FOLLOW: i32 = 2;
/// A turn of the motion above this lays the formation out anew (`DAT_142eb6e44`, π/4).
const TURN_LIMIT: f32 = crate::image::facing::QUARTER_PI;
/// How far ahead of the group its front is looked for (`DAT_14330ebcc`).
const FRONT_AHEAD: f32 = crate::image::movement::LOOK_AHEAD;
/// A total spacing above this lays the formation out anew on a changed count
/// (`DAT_142ec9eb4`).
const RELAYOUT_SPACING: f32 = crate::image::missile::VOLLEY_REACH_MARGIN;
/// The share of the lead's line of sight a follow keeps within (`DAT_142e7f384`).
const FOLLOW_SHARE: f32 = crate::image::group::FRONT_SHARE;
/// A follow's target this near the top path's end (squared, `DAT_14330eb8c`) keeps the move.
const CLOSE: f32 = crate::image::group::CLOSE;
/// The differentiator groups' members the update lists for the members' moves, at most this
/// many (`DAT_143f15860`: `0x8b < n` ends the list).
const DIFFERENTIATORS: usize = 0x8c;
/// The idle timer past which an idle group draws it anew (the immediate `0xbb8`).
pub const IDLE_LIMIT: u32 = 3000;
/// A trade cart's standing and walking sprites are its trade task's while it carries goods
/// (`FUN_14106a940`: its carried attributes, `+0x7a0`, which the slice does not hold).
const TRADE_CART_SPRITES: GenieError =
    GenieError::unsupported("a trade cart's standing or walking sprite (its goods, FUN_14106a940)");

/// The sprites slots 0x660, 0x658 and 0x650 show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shown {
    /// Slot 0x660's standing sprite.
    Standing,
    /// Slot 0x658's walking sprite.
    Walking,
    /// Slot 0x650's running sprite.
    Running,
}

/// The updates of an attack (`FUN_14103f4e0`) and of the other commands on a target
/// (`FUN_141040bd0`), and the attack formation's refill (`FUN_1410163f0`): not modelled.
const OTHER_COMMANDS: GenieError = GenieError::unsupported(
    "a group command's update other than a move or a follow (FUN_14103f4e0, FUN_141040bd0)",
);

impl UnitGroup {
    /// `IsIdle` (`FUN_141017ad0`): idle with flags bit 5 (`cConsiderIdle`), or on a move
    /// (command 1) while it patrols ([`UnitGroup::patrols`]); else not while its motion moves or
    /// waits to move; else idle.
    #[must_use]
    pub fn is_idle(&self) -> bool {
        if self.flags >> 5 & 1 != 0 {
            return true;
        }
        if self.current_command == 1 && self.patrols() {
            return true;
        }
        !(self.motion.moving() || self.motion.waits_to_move())
    }

    /// The group patrols: its top path's flags have bit 0 (`FUN_141017c40`, the path of
    /// `FUN_141046500`) or its flags bit 11 (`FUN_141017c30`).
    #[must_use]
    pub fn patrols(&self) -> bool {
        crate::group::formation::top_path(&self.motion.paths).is_some_and(|p| p.path_flags & 1 != 0)
            || self.flags >> 11 & 1 != 0
    }

    /// The group's vtable slot 8 after its update (`BUnitGroup::updateEnd`, `FUN_141040b50`): a
    /// group whose motion moved (motion flags bit 0) adds its new position to the front of its
    /// stored positions ([`crate::group::PointLine::add_at_front`]); the motion's flags lose bit
    /// 0, and bit 1 with it when set, which sets the group's flags bit 1 (`FUN_14104a240`).
    pub fn update_end(&mut self) {
        let g = self;
        if g.motion.flags & 1 != 0 {
            let [point, forward, up, right] = g.motion.new_position;
            g.stored_positions.add_at_front(crate::group::LinePoint {
                point,
                forward,
                up,
                right,
            });
        }
        let flags = g.motion.flags;
        g.motion.flags &= !1;
        if flags & 2 != 0 {
            g.motion.flags &= !3;
            g.flags |= 2;
        }
    }
}

#[cfg(test)]
mod tests;
