//! The Hunt action (`TRIBE_Action_Hunt`, type 110, vtable `0x142ed70f8`): a villager sent to a
//! tree (or an animal) with a gather task attacks it first (state 12, an [`Attack`] sub-action);
//! once it is dead the villager walks to it (state 4, a `MoveTo` sub-action), faces it (6) and
//! gathers (7). The drop-off (8 on) is not modelled yet.
//!
//! 158041: constructor `FUN_140ac2dd0`, start (slot 5) `0x140ac49b0`, update (slot 6)
//! `0x140ac7a00`, `set_state` (slot 16) `0x140ac60e0`.

use crate::combat::attack::{Arms, Attack, Engagement};
use crate::move_to::{MoveTo, MoveWait};
use crate::sprite::Graphic;

mod drop_off;
mod gathering;
mod herding;
mod sprites;
mod start;
mod update;

#[cfg(test)]
use self::drop_off::DROP_WAIT;
pub use self::drop_off::Dropped;
pub use self::sprites::{HuntSprites, Shown, walk_sprite};
pub(crate) use self::sprites::{asked_drop_off, resource_kind, stand_sprite};
pub use self::start::{PointWalk, PointWalker};
#[cfg(test)]
use crate::combat::damage::Victim;
#[cfg(test)]
use crate::move_to::Walk;
#[cfg(test)]
use crate::movement::Mover;

/// The Hunt action's type, which word 9 adds.
pub const HUNT: i16 = 110;

/// The states the slice models: 4 walking to a dead target, 6 facing it, 7 gathering from it, 8
/// taking the load to its drop site, 12 attacking it (`0xc`).
pub const WALK: u8 = 4;
/// State 6: facing the dead target.
pub const FACE: u8 = 6;
/// State 7: gathering from it.
pub const GATHER: u8 = 7;
/// State 8: taking the load to its drop site.
pub const DROP: u8 = 8;
/// State 10 (`0xa`): leading its own livestock to the drop site, it waits for the animal to stop
/// there.
pub const LEAD: u8 = 10;
/// State 12 (`0xc`): attacking it.
pub const ATTACK: u8 = 12;
/// State 13: the hunt failed; its next update tells its unit AI (notify 0x1f9) and ends it.
pub const FAILED: u8 = 13;
/// State 2: the hunt is done with its target; it waits (a tree's target class would search
/// again, `set_state` 3).
pub const IDLE: u8 = 2;
/// State 3: searching: it told its unit AI to look for the next target (notify 0x202), again
/// every 10 seconds.
pub const SEARCH: u8 = 3;
/// State 15 (`0xf`): walking to a point a move gave it (its slot 0x40).
pub const WALK_POINT: u8 = 15;
/// State 11 (`0xb`): walking to the point after the target went on the way to it; its end
/// searches (`set_state` 3).
pub const WALK_SEARCH: u8 = 11;
/// The class of trees, for which the civilian unit AI's slot 0x1b0 (`FUN_140c1c4f0`) gives 0: a
/// hunt whose target went searches for the next one (`FUN_140ac4670`).
pub const TREE_CLASS: i32 = 15;
/// A building whose top action is this (a farm at work) takes a hunt sent to it to `set_state` 15.
const FARM_AT_WORK: i16 = 0x7d;

/// A `MoveTo` starts in place within this gap of its target (`0x140aa2100`).
const ARRIVED: f32 = 0.01;
/// Own livestock farther than this squared from its drop site is led there first
/// (`DAT_142ecccc4`).
const HERD_REACH: f32 = crate::math::image::hunt::HERD_REACH;
/// Own livestock this far squared from its drop site or farther is slaughtered where it stands
/// (`DAT_142eb0f64`).
const HERD_FAR: f32 = 900.0;
/// A herder within this squared gap of the livestock it leads waits for it again
/// (`DAT_142ed7180`).
const LEAD_REACH: f32 = 2.25;
/// The wait of a herder leading livestock (`set_state(10)`'s timer, `0xbf800000`).
const LEAD_WAIT: f32 = -1.0;
/// A face on dead livestock beyond the task's range plus this walks again (`DAT_142eb5690`).
const GAP: f32 = crate::math::image::hunt::GAP;
/// Below this a target's or a load's remainder counts as nothing (`DAT_142eb5690`).
const REMAINDER: f32 = crate::math::image::hunt::REMAINDER;
/// A target's last bit adds its master's first storage amount times this (`DAT_142eb1e9c`).
const LAST_BIT: f32 = crate::math::image::hunt::LAST_BIT;
/// A task whose `attribute_type4` is this (`0x124`) stops its target's decay under player
/// attribute 292 (`FUN_140ac44b0`).
const DECAY_TASK: i16 = 0x124;

/// A building's master type (`'P'`).
const BUILDING: u8 = 80;
/// The range of the walk to the hunter's own player's livestock (`DAT_142eb4838`).
const OWN_LIVESTOCK_RANGE: f32 = crate::math::image::hunt::OWN_LIVESTOCK_RANGE;
/// The villagers' class, whose `MoveTo` waits once when the motion step gives up.
const VILLAGER_CLASS: i16 = 4;
/// Classes the hunt treats apart: livestock (58, `-0xe72`) and 49 (`-0xe7b`).
pub const LIVESTOCK: i16 = 58;
/// The predators' class, whose hunt ends on its own terms (`-0xea2` raw, `0x140ac85fd`).
const PREDATOR: i16 = 10;
const CLASS_49: i16 = 49;

/// The Hunt action of a gatherer (`TRIBE_Action_Hunt`, type 110).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HuntAction {
    /// The target object (`+0x20`, its id at `+0x30`).
    pub target: i32,
    /// The action state (`+0x18`), which word 9 adds.
    pub state: u8,
    /// `+0x38`..`+0x40`: where the target stood when the hunt took it.
    pub point: [f32; 3],
    /// `+0x68`: the target's class.
    pub target_class: i16,
    /// `+0x48`: its task.
    pub task: HuntTask,
    /// The sub-action of state 12.
    pub attack: Option<Attack>,
    /// The sub-action of state 4: a `MoveTo` to the target within `walk_range`, quiet (its
    /// `+0x60` and `+0x61` set: its end changes neither the sprite nor the unit's notifies).
    pub move_to: Option<MoveTo>,
    /// The `MoveTo`'s range: the task's (`+0x30`) to a dead target, `OWN_LIVESTOCK_RANGE` to its
    /// own player's livestock.
    pub walk_range: f32,
    /// `+0x70`: the drop site its own livestock is led to, −1 for none yet.
    pub drop_site: i32,
    /// `+0x28` (its id at `+0x34`), the second target: the drop site the load goes to in state 8,
    /// −1 for none.
    pub carry_to: i32,
    /// `+0x44`: the state's timer (state 8 waits with it below 0 before it tries again).
    pub timer: f32,
    /// Its `MoveTo`'s wait after the motion step gave up.
    pub move_wait: MoveWait,
    /// What it keeps of the livestock it leads to its drop site.
    pub lead: Lead,
}

/// What a hunt keeps of the livestock it leads to its drop site (`TRIBE_Action_Hunt`'s
/// `moved_sheep` `+0x6c`, `sheep_still_time` `+0x74` and `sheep_start_x`, `_y` `+0x78`, `+0x7c`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Lead {
    /// `moved_sheep`: the hunt sent its livestock off once.
    pub moved: bool,
    /// `sheep_still_time`, which `set_state(10)` clears.
    pub still_time: i16,
    /// `sheep_start_x`, `_y`: where the animal stood when the hunt last looked.
    pub start: [f32; 2],
}

impl Lead {
    /// The constructor's (`FUN_140ac2dd0`): nothing moved or still, the start at (−1, −1)
    /// (`0xbf800000`).
    pub const NEW: Self = Self {
        moved: false,
        still_time: 0,
        start: [-1.0, -1.0],
    };
}

/// What the drop-off reads of its drop site on arrival: that the site takes what the unit carries
/// (`FUN_140fdd190`) and the gap between the two footprints squared (`FUN_14118f8c0`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DropOff {
    /// The site takes what the unit carries (`FUN_140fdd190`).
    pub takes: bool,
    /// The gap between the two footprints squared (`FUN_14118f8c0`).
    pub gap_squared: f32,
}

/// What `set_state(6)` reads of a live livestock target (`0x140ac60e0` case 6).
#[derive(Clone, Copy, Debug)]
pub struct Herd {
    /// The target's player is the hunter's.
    pub own: bool,
    /// For its own livestock, the drop site (the hunt's, else the nearest that takes livestock,
    /// `FUN_1411671e0`) and the gap between it and the target squared (`FUN_14118f8c0`).
    pub drop_site: Option<(i32, f32)>,
    /// The Attack `set_state(12)` starts.
    pub engagement: Engagement,
    /// The Attack's sprites and frames.
    pub arms: Arms,
    /// The target's top action has neither a target nor a point (its slots 0x190 and 0x288,
    /// `FUN_14106d090`, `FUN_141082dc0`): it stands where it was sent.
    pub idle: bool,
    /// The hunter is of type 70 and its master has a special graphic (`+0x8e8`): the sprite
    /// leading shows.
    pub special_graphic: Option<Graphic>,
}

/// What the hunt reads of its task (`+0x48`): its id in the master's list (`+0xa`; the work command
/// keeps a hunt on the task it finds), the work range (`+0x30`) and the resource it gathers
/// (`attribute_type`, `+0x20`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HuntTask {
    /// Its id in the master's task list (`+0xa`).
    pub id: i16,
    /// The work range (`+0x30`).
    pub range: f32,
    /// The resource it gathers (`attribute_type`, `+0x20`).
    pub resource: i16,
    /// Its moving, proceeding, working and carrying sprites (`+0x40`..`+0x58`).
    pub sprites: [i16; 4],
    /// `+0x28`, `work_val1`: what one work gathers per second and work rate.
    pub work_value: f32,
    /// `+0x22`, `attribute_type2`: the player's attribute that scales the gathering, -1 none.
    pub productivity: i16,
    /// `+0x26`, `attribute_type4`.
    pub kind4: i16,
}

/// What the gathering reads of its unit and changes besides the attack's [`Strike`].
///
/// [`Strike`]: crate::combat::attack::Strike
#[derive(Debug)]
pub struct Gatherer<'a> {
    /// `+0x308`, `+0x310`: what the unit carries.
    pub held_kind: &'a mut i16,
    /// `+0x310`: how much.
    pub held: &'a mut f32,
    /// The master's work rate (`+0x798`) plus the unit's powerup (slot 0x6e8, `+0xa30`).
    pub work_rate: f32,
    /// The master's `attribute_max_amount` (`+0x61c`): what the unit carries at most.
    pub capacity: f32,
    /// The player's attribute the task's `attribute_type2` names, when above 0; else 1.
    pub productivity: f32,
    /// The target's master's class and its first storage amount (`+0x610`).
    pub target_class: i16,
    /// The target's master's first storage amount (`+0x610`).
    pub target_storage: f32,
    /// The player's attribute 292 is 1: a task whose `attribute_type4` is 0x124 stops the
    /// target's decay (`FUN_140ac44b0`).
    pub stops_decay: bool,
    /// The unit AI's target class (`+0x4c`, `FUN_140c4cc50`), None for a unit without one.
    pub ai_class: Option<i32>,
    /// The caller ran the search for the next tree a lost target asks ([`HuntAction::update`]'s
    /// `FUN_140ac4670`, `Unit::hunt_tree_search`) and found none.
    pub tree_searched: bool,
    /// The game's setting [`crate::settings::Settings::predator_hunts_end`].
    pub predator_hunts_end: bool,
    /// The sprites the hunt shows.
    pub sprites: HuntSprites,
    /// Out: the hunt let its target go and asks for `set_state(3)`'s search (the caller's: the
    /// drop site, the notify, the sprite, the timer).
    pub searches: bool,
    /// Out: the hunt sends its livestock to this drop site (the target's slot 0x138) with the
    /// herd around it (`FUN_140ac4c60`), the caller's.
    pub leads: Option<i32>,
    /// Out: the Attack's `MoveTo` failed (code 3 in state 12): `FUN_140ac4670` with −1 and 5
    /// takes the next tree for a tree's class (the caller's, `Unit::hunt_tree_search`), else (or
    /// with none found) `set_state(3)` with the target kept (the caller's search).
    pub attack_failed: bool,
    /// Out: the target the hunt let go (slot 0x68 with none), which the caller releases.
    pub let_go: Option<i32>,
}

/// What the hunt reads of its target when it starts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quarry {
    /// The target.
    pub id: i32,
    /// Its place.
    pub position: [f32; 3],
    /// Its master's class and type, its state (`+0x640`) and owner.
    pub class: i16,
    /// Its master's type.
    pub kind: u8,
    /// Its state (`+0x640`).
    pub state: u8,
    /// Its owner.
    pub player: usize,
    /// What it holds (`+0x310`).
    pub amount: f32,
    /// The hunter's player counts the target's player as its own or allied (`FUN_141663d90` with
    /// the ally flag).
    pub allied: bool,
    /// Its top action's type (a building's `+0x880` list).
    pub top_action: Option<i16>,
    /// The hunter's master has its master (a building annex's main building's, `+0xbb8`) among
    /// its drop sites (`FUN_14109d4f0`), which the start reads of its own player's object.
    pub drop_site: bool,
}

/// Food and the food kinds of carcasses and fish (0, 15 to 17): a hunter keeps what it carries
/// of one for another.
fn food(kind: i16) -> bool {
    kind == 0 || (15..18).contains(&kind)
}

#[cfg(test)]
pub(crate) mod tests;
