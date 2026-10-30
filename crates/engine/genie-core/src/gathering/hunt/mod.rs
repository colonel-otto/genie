//! The Hunt action (`TRIBE_Action_Hunt`, type 110, vtable `0x142ed70f8`): a villager sent to a
//! tree (or an animal) with a gather task attacks it first (state 12, an [`Attack`] sub-action);
//! once it is dead the villager walks to it (state 4, a `MoveTo` sub-action), faces it (6) and
//! gathers (7), then carries the load to a drop site (8). A hunter of its own livestock leads the
//! animal toward the drop site first (state 10).
//!
//! 158041: constructor `FUN_140ac2dd0`, start (slot 5) `0x140ac49b0`, update (slot 6)
//! `0x140ac7a00`, `set_state` (slot 16) `0x140ac60e0`.

use crate::combat::attack::{Arms, Attack, AttackReads, Engagement};
use crate::map::Point;
use crate::map::Position;
use crate::movement::move_to::{MoveDefaults, MoveToAction};
use crate::objects::ObjectId;
use crate::objects::masters::ClassId;
use crate::objects::masters::MasterKind;
use crate::objects::masters::TaskId;
use crate::objects::sprite::Graphic;
use crate::players::PlayerId;
use genie_diff::Diff;

/// A hunter's update without its Hunt: the caller's test.
pub(super) const NO_HUNT: crate::GenieError =
    crate::GenieError::unreachable("a hunter's update without its Hunt");

mod drop_off;
mod gathering;
mod herding;
mod hunter;
mod hunter_strike;
pub(crate) mod next_tree;
mod sprites;
mod start;
pub(crate) mod unit;
mod update;
mod walks;

#[cfg(test)]
use self::drop_off::DROP_WAIT;
pub use self::drop_off::Dropped;
pub use self::sprites::{HuntSprites, walk_sprite};
pub(crate) use self::sprites::{asked_drop_off, resource_kind, stand_sprite};
pub use self::start::{HuntMove, HuntRetarget, HuntStart, PointWalk, PointWalker};
#[cfg(test)]
use crate::combat::damage::Victim;
#[cfg(test)]
use crate::movement::Mover;
pub use crate::movement::move_to::Shown;

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

/// The range of the walk to the hunter's own player's livestock (`DAT_142eb4838`).
const OWN_LIVESTOCK_RANGE: f32 = crate::math::image::hunt::OWN_LIVESTOCK_RANGE;

/// The Hunt action of a gatherer (`TRIBE_Action_Hunt`, type 110).
#[derive(Clone, Copy, Debug, PartialEq, Diff)]
pub struct HuntAction {
    /// The target object (`+0x20`, its id at `+0x30`).
    pub target: Option<ObjectId>,
    /// The action state (`+0x18`), which word 9 adds.
    pub state: u8,
    /// `+0x38`..`+0x40`: where the target stood when the hunt took it.
    pub point: Position,
    /// `+0x68`: the target's class.
    pub target_class: Option<ClassId>,
    /// `+0x48`: its task.
    pub task: HuntTask,
    /// The sub-action of state 12.
    pub attack: Option<Attack>,
    /// The sub-action of states 4, 8, 11 and 15: the quiet `MoveTo` (the one component,
    /// [`MoveToAction`]) to the target within the task's range (`OWN_LIVESTOCK_RANGE` to its own
    /// player's livestock), to the drop site, or to the point, which `set_state` makes.
    pub move_to: Option<MoveToAction>,
    /// `+0x70`: the drop site its own livestock is led to, −1 for none yet.
    pub drop_site: i32,
    /// `+0x28` (its id at `+0x34`), the second target: the drop site the load goes to in state 8,
    /// −1 for none.
    pub carry_to: Option<ObjectId>,
    /// `+0x44`: the state's timer (state 8 waits with it below 0 before it tries again).
    pub timer: f32,
    /// What it keeps of the livestock it leads to its drop site.
    pub lead: Lead,
}

/// What a hunt keeps of the livestock it leads to its drop site (`TRIBE_Action_Hunt`'s
/// `moved_sheep` `+0x6c`, `sheep_still_time` `+0x74` and `sheep_start_x`, `_y` `+0x78`, `+0x7c`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Diff)]
pub struct Lead {
    /// `moved_sheep`: the hunt sent its livestock off once.
    pub moved: bool,
    /// `sheep_still_time`, which `set_state(10)` clears.
    pub still_time: i16,
    /// `sheep_start_x`, `_y`: where the animal stood when the hunt last looked.
    pub start: Point,
}

impl Lead {
    /// The constructor's (`FUN_140ac2dd0`): nothing moved or still, the start at (−1, −1)
    /// (`0xbf800000`).
    pub const NEW: Self = Self {
        moved: false,
        still_time: 0,
        start: Point::new(-1.0, -1.0),
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
#[derive(Clone, Copy, Debug, PartialEq, Diff)]
pub struct HuntTask {
    /// Its id in the master's task list (`+0xa`).
    pub id: TaskId,
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

/// What the hunt's update reads of its unit, its master, its player and the game besides its
/// Attack's reads, copied as the update starts ([`crate::objects::unit::Unit::hunt_reads`]).
#[derive(Clone, Copy, Debug)]
pub(crate) struct HuntReads {
    /// What its Attack reads ([`AttackReads::of`]).
    pub(crate) attack: AttackReads,
    /// What the `MoveTo`s' constructors take for a sprite when the task gives none
    /// (`FUN_140aa0d30`, `FUN_140aa0df0`).
    pub(crate) defaults: MoveDefaults,
    /// The master's work rate (`+0x798`) plus the unit's powerup (slot 0x6e8, `+0xa30`).
    pub(crate) work_rate: f32,
    /// The master's `attribute_max_amount` (`+0x61c`): what the unit carries at most.
    pub(crate) capacity: f32,
    /// The player's attribute the task's `attribute_type2` names, when above 0; else 1.
    pub(crate) productivity: f32,
    /// The player's attribute 292 is 1: a task whose `attribute_type4` is 0x124 stops the
    /// target's decay (`FUN_140ac44b0`).
    pub(crate) stops_decay: bool,
    /// The unit AI's target class (`+0x4c`, `FUN_140c4cc50`), None for a unit without one.
    pub(crate) ai_class: Option<i32>,
    /// The caller ran the search for the next tree a lost target asks ([`HuntAction::update`]'s
    /// `FUN_140ac4670`, `Unit::hunt_tree_search`) and found none.
    pub(crate) tree_searched: bool,
    /// The game's setting [`crate::world::settings::Settings::predator_hunts_end`].
    pub(crate) predator_hunts_end: bool,
    /// The sprites the hunt shows.
    pub(crate) sprites: HuntSprites,
    /// What `set_state(6)` on live livestock and the lead read ([`Herd`]); none in the other
    /// states.
    pub(crate) herd: Option<Herd>,
}

/// What follows the hunt's update once the hunt is back in its unit's list
/// ([`HuntAction::update`]): the parts of the unit's own that read the hunt there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HuntNext {
    /// Nothing more.
    Stay,
    /// The gathering ended: `set_state(8)`, its drop site search
    /// ([`crate::objects::unit::Unit::hunt_drop_off`]).
    Drop,
    /// The hunt let its target go and searches: `set_state(3)`
    /// ([`crate::objects::unit::Unit::hunt_set_search`]).
    Search,
    /// The Attack's `MoveTo` failed (code 3 in state 12): `FUN_140ac4670` with −1 and 5 takes
    /// the next tree for a tree's class, else (or with none found) `set_state(3)` with the
    /// target kept ([`crate::objects::unit::Unit::hunt_attack_failed`]).
    AttackFailed,
}

/// What the hunt reads of its target when it starts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quarry {
    /// The target.
    pub id: i32,
    /// Its place.
    pub position: Position,
    /// Its master's class and type, its state (`+0x640`) and owner.
    pub class: ClassId,
    /// Its master's type.
    pub kind: MasterKind,
    /// Its state (`+0x640`).
    pub state: u8,
    /// Its owner.
    pub player: PlayerId,
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
