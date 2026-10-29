//! A builder's Build action (`TRIBE_Action_Build`, type 101, vtable `0x142ed6628`) up to the
//! builder's walk: the action starts in state 4 with a `MoveTo` sub-action (`RGE_Action_Move_To`,
//! type 1) toward the foundation; a builder already at the foundation works in its first update
//! (state 6). A walking builder's `MoveTo` runs the motion step ([`crate::movement`]).
//!
//! A builder at a foundation faces it and joins its work (state 7): at a foundation no one started
//! it first checks the footprint, and a builder standing on it walks to a free tile beside it
//! (state 223) and comes back (state 4). The work adds to the foundation's construction each
//! update ([`Construction::work`]); the first work starts it: the foundation gets its obstruction
//! record. The work that reaches the build time completes the building ([`Construction::complete`])
//! and ends the builder's work (state 224); a builder whose unit AI holds queued orders then lets
//! the Build end (state 1) and the AI runs its queue.

use self::update::drop_done_orders;
use std::collections::BTreeMap;

use crate::GenieError;
use crate::map::spatial::Obstruction;
use crate::movement::move_to::{MoveDefaults, MoveTo, MoveWait};
use crate::objects::lifecycle::Lifecycle;
use crate::objects::masters::{Master, Task};
use crate::unit_ai::OrderKind;
use genie_diff::Diff;

mod clearing;
mod construction;
mod drop_off;
pub(crate) mod more_work;
mod placing;
mod states;
pub(crate) mod unit;
mod unit_ai_side;
mod update;
mod walk_ends;
pub(crate) mod world_side;

pub(crate) use self::clearing::{CLEARING_REACH, clearing_objects, footprint_clearing};
pub use self::clearing::{ClearLook, Cleared, ClearingSite};
#[cfg(test)]
use self::clearing::{GATE, TREE};
pub use self::drop_off::DropSearch;
#[cfg(test)]
use self::placing::beside_diagonal;
pub(crate) use crate::construction::build::world_side::{NO_CONSTRUCTION, NOT_HELD, held};
#[cfg(test)]
use crate::map::spatial::Spatial;
#[cfg(test)]
use crate::pathing::Pathing;

/// The Build action's type, which word 9 adds.
pub const BUILD: i16 = 101;

/// The gap below which a `MoveTo` has arrived (`DAT_142eb99cc`).
const ARRIVED: f32 = crate::math::image::build::ARRIVED;
/// The villager's class (4, `-0xea8` encoded): Build's walk off the footprint returns at once.
const VILLAGER: i16 = 4;
/// A builder within this of its foundation's footprint has reached it (`DAT_142eb5690`).
const REACH: f32 = crate::math::image::build::REACH;
/// A player's sight of an object it sees (slot 0x1e0).
const SEEN: u8 = 0xf;
/// A foundation of another player than the builder's, which the slice does not hand the Build.
const FOREIGN_SITE: GenieError = GenieError::unsupported("a foundation of another player");
/// The Build's foundation the world no longer holds where its place is read.
const SITE_GONE: GenieError = GenieError::unsupported("a foundation that is gone");
/// A builder without movement state.
const BUILDER_UNMOVING: GenieError = GenieError::unsupported("a builder without movement state");
/// A foundation its owner sees only through a ghost of it on its tile (`FUN_1411d3280`'s last
/// test), which the Build's world does not hold.
const UNSEEN_SITE: GenieError =
    GenieError::unsupported("a foundation its owner sees only through its ghosts (slot 0x1e0)");

/// The Build action of a builder (`TRIBE_Action_Build`, type 101).
#[derive(Clone, Copy, Debug, PartialEq, Diff)]
pub struct BuildAction {
    /// The foundation's id.
    pub target: i32,
    /// The action state (`+0x18`), which word 9 adds.
    pub state: u8,
    /// Its `MoveTo` sub-action's state.
    pub move_to: Option<MoveTo>,
    /// The point the `MoveTo` walks to (its `+0x38`..`+0x40`), when not to the foundation.
    pub goal: Option<[f32; 3]>,
    /// The `MoveTo`'s target (its `+0x30`), −1 for a `MoveTo` to a point: the foundation its
    /// `set_state(4)` walks to, which the `MoveTo` keeps when the Build lets the foundation go.
    pub walk_target: i32,
    /// `+0x38`..`+0x40`, the action's own point: the foundation's place from the constructor
    /// (`FUN_140aad4e0`) and each `set_state(4)` with a target, −1 each from `set_state(2)`;
    /// `set_state(11)` walks there.
    pub point: [f32; 3],
    /// The task it builds with.
    pub task: BuildTask,
    /// `+0x68`, `added_to_num_builders`: the builder counts among the foundation's builders.
    pub added: bool,
    /// `+0x44`, the action's timer: state 222's wait (a second from its `set_state`).
    pub timer: f32,
    /// Its `MoveTo`'s wait after the motion step gave up (`+0x6c`..`+0x74`).
    pub move_wait: MoveWait,
    /// The sprite its `MoveTo` was made with (`+0x58` as the constructor takes it): the task's
    /// moving sprite, on a drop-off the drop task's carrying one first; None for none, for which
    /// the constructor takes its default ([`BuildAction::move_shown`]).
    #[diff(
        gap = "the game's MoveTo keeps the resolved sprite (+0x58, FUN_140aa0df0), genie the argument"
    )]
    pub move_sprite: Option<i16>,
    /// `+0x28` (its id at `+0x34`), the second target: the drop site of its drop-off (state 8,
    /// `set_state(8)`'s search), −1 for none.
    pub site: i32,
    /// The range of its drop-off's `MoveTo` (that `MoveTo`'s `+0x68`): the drop site's task's
    /// for the builder (`+0x30`), else the Build task's.
    #[diff(
        gap = "the MoveTo's range (+0x68) is held per owning action; the save's sub-action holds one"
    )]
    pub drop_range: f32,
}

/// What a builder's Build task gives its work: the task's work range (`+0x30`; its `MoveTo` to the
/// foundation stops this near) and work value (`+0x28`, `work_val1`), and the builder's work rate:
/// its master's `work_rate` (`+0x798`) plus its own `mWorkRatePowerup` (`+0xa30`, slot 0x6e8).
#[derive(Clone, Copy, Debug, PartialEq, Diff)]
pub struct BuildTask {
    /// The task's work range.
    pub range: f32,
    /// The task's work value.
    pub work_value: f32,
    /// The builder's work rate: its master's `work_rate` (`+0x798`) plus its `mWorkRatePowerup`
    /// (`+0xa30`).
    pub work_rate: f32,
    /// The task's resource (`attribute_type`, `+0x20`, −1 for any): `UNIT_ORDER`'s drop-off
    /// (slot 0x78) asks a builder carrying another one to its unit AI instead.
    pub resource: i16,
    /// The task's id in its master's list (`+0xa`), which a master's upgrade finds the new
    /// master's task by (−1 for none).
    pub id: i16,
    /// The task's moving sprite (`+0x40`), which it gives its `MoveTo`s; None for none.
    pub moving: Option<i16>,
}

impl BuildTask {
    /// The Build task `task` of a builder whose work rate is `work_rate`.
    #[must_use]
    pub const fn of(task: &Task, work_rate: f32) -> Self {
        Self {
            range: task.work_range,
            work_value: task.work_val1,
            work_rate,
            resource: task.attribute_type,
            id: task.id,
            moving: crate::objects::sprite::sprite_id(task.sprites[0]),
        }
    }
}

/// A building under construction (`TRIBE_Building_Object`), with what its master says of it.
#[derive(Clone, Copy, Debug, PartialEq, Diff)]
pub struct Construction {
    /// `+0xc50`, `build_pts`: the seconds of work it has had.
    pub progress: f32,
    /// `+0xc60`, `desolid_flag`: no builder has started it; it has no obstruction record.
    pub desolid: bool,
    /// `+0x9e8`, `num_builders`.
    pub builders: u8,
    /// Its master's hit points (`+0x530`) and build time (`+0x898`, the first train location's
    /// train time; 0 without one).
    pub hit_points: i16,
    /// Its master's build time (the first train location's train time; 0 without one).
    pub build_time: i16,
    /// The record its start adds: its master's obstruction kind and class.
    pub record: Obstruction,
    /// `+0xc61`, `snow_flag`: it was completed on snow (its decal is not held).
    #[diff(
        gap = "the teardowns' clearing (FUN_1411d5600, FUN_1411ad250) is not ported; the game's flag has three values"
    )]
    pub snow: bool,
    /// `+0x788`, `sprite_override_id`, is not −1.
    pub sprite_override: bool,
    /// Its construction started in this update: its owner's watched list takes it
    /// (`FUN_1411e5670`'s `FUN_141090430`, the caller's).
    pub started: bool,
}

/// What a builder's work reaches of its foundation: the split borrow [`Construction::work`] and
/// [`Construction::complete`] take, built where the work runs.
#[derive(Debug)]
pub struct Site<'a> {
    /// The foundation.
    pub id: i32,
    /// Its life cycle.
    pub life: &'a mut Lifecycle,
    /// Its construction.
    pub construction: &'a mut Construction,
    /// The foundation's movement state (its last move time), if the slice holds it.
    pub mover: Option<&'a mut crate::movement::Mover>,
    /// Its missiles per volley (`+0xa00`), which its construction's start sets to its master's.
    pub firepower: Option<&'a mut f32>,
    /// Its `mROOAlreadyCalled` (`+0x686`), which its stack unit's completion sets.
    pub roo_called: Option<&'a mut bool>,
    /// Its own store (`+0xae0`), which its storage entries of mode 16 fill at its completion.
    pub store: Option<&'a mut Vec<(i16, f32)>>,
    /// What its completion reads ([`Construction::complete`]).
    pub completion: Completion<'a>,
    /// What its work (`FUN_1411cc430`) reaches besides it: its annexes of a building type,
    /// which it starts first, and the main building that takes the work when it is an annex
    /// ([`WorkChain`]).
    pub chain: WorkChain<'a>,
}

/// What a foundation's work (`FUN_1411cc430`) reaches besides it ([`Construction::work`]).
#[derive(Debug, Default)]
pub struct WorkChain<'a> {
    /// Its annexes (`+0xbc0`..`+0xbd8`) of a building type (above 0x4f), in their order.
    pub annexes: Vec<AnnexStart<'a>>,
    /// The building the work goes to instead while the foundation is an annex of a main
    /// building in state 0 with at least 1 hit point (that one's own main alike).
    pub main: Option<MainWork>,
}

/// An annex the work on its building starts first while it is desolid (`+0xc60`):
/// `FUN_1411cd0d0` (after the builder's update, as the building's own), then `FUN_1411e5670`
/// ([`Construction::begin`]; its missiles per volley become its master's).
#[derive(Debug)]
pub struct AnnexStart<'a> {
    /// The annex.
    pub id: i32,
    /// Its construction.
    pub construction: &'a mut Construction,
    /// Its movement state, if the slice holds it.
    pub mover: Option<&'a mut crate::movement::Mover>,
    /// Its missiles per volley (`+0xa00`).
    pub firepower: &'a mut f32,
    /// Its master's volley fire.
    pub volley: f32,
}

/// The main building an annex's work goes to: its construction and hit points as they stand,
/// from which the work's result follows. The work itself (the annexes' starts on the way, the
/// main building's start, work and completion) is the caller's after the builder's update
/// ([`crate::world::World::work_building`] on the annex, [`Worked::Main`]).
#[derive(Clone, Copy, Debug)]
pub struct MainWork {
    /// Its construction.
    pub construction: Construction,
    /// Its hit points.
    pub hp: f32,
}

/// What a building's completion reads besides its construction.
#[derive(Clone, Copy, Debug)]
pub struct Completion<'a> {
    /// The building's master.
    pub master: Option<&'a Master>,
    /// Another object is its annex (its `+0xbc0` list).
    pub annexed: bool,
    /// Some player's attribute 204 is not 0 (`FUN_14152bcb0`).
    pub attribute_204: bool,
    /// The game's setting at `+0x680` ([`crate::world::World::mode_680`]).
    pub mode_680: i32,
    /// The owner's attributes 237, 238 and 239 (`player+0x768`..`+0x778`): a farm's bonus when
    /// built, its amount, its resource and a master near which it is given
    /// ([`Completion::first_part`]).
    pub farm_bonus: [f32; 3],
    /// The owner's masters, whose table `FUN_141659d20` reads.
    pub masters: Option<&'a crate::objects::masters::Masters>,
    /// The owner is a computer player (`FUN_141659cf0`).
    pub computer: bool,
    /// The building has a unit AI (`+0x6b0`): a mule cart's.
    pub ai: bool,
    /// The building is an annex (`+0xbb8`, its main building's): no paid cost goes to attributes
    /// 98 and 165, its tiles keep their links.
    pub main: bool,
    /// The world's time (`FUN_1415300d0`), which a construction's start reads.
    pub now: u32,
    /// The building stands on mangrove or rice terrain (`+0xcd0`, `FUN_141188db0` at its
    /// constructor): master 1889's stack unit is the pasture 1893 then.
    pub rice: bool,
}

/// What the completion's first part (`FUN_1411c6bd0`) gives its caller: the attributes its owner
/// gains (a farm's bonus) and whether a mule cart's search for resources follows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FirstPart {
    /// The attributes its owner gains, in order.
    pub attributes: Vec<(i16, f32)>,
    /// The building's master has a task of action type 0x99 and the building a unit AI: its
    /// search for resources follows ([`crate::world::World::cart_search`]).
    pub cart: bool,
    /// A pasture (1893, 1897) whose build points reached its build time: its livestock follow
    /// after the attributes (`FUN_1411c8410`, [`crate::world::World::setup_pasture`]).
    pub pasture: bool,
}

impl Completion<'_> {
    /// The completion's first part (`FUN_1411c6bd0`), which the building's `set_state` runs from
    /// state 0 to any state of 2 or more: a farm (class 49) whose build points reached its build
    /// time (COMISS, JC: a NaN does not) gets its owner's bonus when attributes 238 (a resource)
    /// and 239 (a master), cut to integers (CVTTSS2SI), are both 0 or more (`FUN_1411c66a0`): a
    /// master 239 the owner has (`FUN_141659d20`) gives it only near the farm
    /// (`FUN_1411d3cd0`, not modelled); a resource up to the owner's attribute count gets
    /// attribute 237's amount (slot 0xf8 with 1 and 1); a pasture (1893, 1897) then fills
    /// ([`FirstPart::pasture`], the caller's); a master with a task of action type 0x99 and a unit AI
    /// runs its cart's search (the tail's game mode 4 test, `FUN_140c87f20`, holds in a game).
    /// Not modelled: a computer player's (`FUN_1411e7ca0`) and an overridden sprite's master
    /// copy (`+0x788`). Not held: the owner's `+0x137c` (cleared for master 109), the object
    /// tracker (`FUN_14104eac0`, `FUN_14104faf0`) and the farm's `+0xcac`.
    ///
    /// # Errors
    ///
    /// What it does not model.
    pub fn first_part(&self, c: &Construction) -> Result<FirstPart, GenieError> {
        use crate::math::conversions::cvttss2si;
        let master = self.master.ok_or(GenieError::unsupported(
            "a completion of a master the slice does not have",
        ))?;
        if self.computer {
            return Err(GenieError::unsupported(
                "a computer player's building part (FUN_1411e7ca0)",
            ));
        }
        if c.sprite_override {
            return Err(GenieError::unsupported(
                "a completion of an overridden sprite",
            ));
        }
        let mut part = FirstPart::default();
        if master.class == FARM && f32::from(c.build_time) <= c.progress {
            let [amount, resource, near] = self.farm_bonus;
            let (resource, near) = (cvttss2si(resource), cvttss2si(near));
            if 0 <= resource && 0 <= near {
                let has = i16::try_from(near)
                    .ok()
                    .and_then(|m| self.masters?.get(m))
                    .is_some();
                if has {
                    return Err(GenieError::unsupported(
                        "a farm's bonus near a master (FUN_1411d3cd0)",
                    ));
                }
                part.attributes.push((resource as i16, amount));
            }
            // the game mode is not the editor's (4) in a game
            part.pasture = matches!(master.id, 1893 | 1897);
        }
        part.cart = self.ai && master.tasks.iter().any(|t| t.action_type == CART_TASK);
        Ok(part)
    }
}

/// What a builder's work did to its foundation ([`Construction::work`]).
#[derive(Clone, Debug, PartialEq)]
pub enum Worked {
    /// The construction goes on.
    Going,
    /// The foundation was no longer under construction (state not 0, or no hit points).
    Done,
    /// This work completed it.
    Completed(Built),
    /// The foundation is an annex whose main building takes the `amount` seconds of work, the
    /// caller's after the update ([`WorkChain::main`]); `going` while its construction goes on
    /// (the routine's 0).
    Main {
        /// The seconds of work.
        amount: f32,
        /// The main building's construction goes on.
        going: bool,
    },
}

/// The action type of the mule cart's tasks (its moves to resources, `0x99`): the action
/// factory makes no action of it.
pub const CART_TASK: i16 = 0x99;

/// What a completion gives its owner ([`Construction::complete`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Built {
    /// The attributes it gains, (attribute, amount), in the order the game adds them.
    pub attributes: Vec<(i16, f32)>,
    /// The event its owner's history records with the owner's id (`FUN_141560ae0`, the
    /// caller's): the castle's (6) and the wonder's (4), none for other masters.
    pub history: Option<u8>,
    /// The master's technology (`+0x98c`, the building's `tech`), which the completion researches
    /// last (`FUN_1413cdcd0` without a building): the caller's.
    pub tech: Option<i16>,
    /// The building's master has a task of action type 0x99 (a mule cart, `FUN_1415c60b0`): its
    /// unit AI's search for resources follows (`FUN_1411c6bd0`'s tail, the caller's), in the
    /// state the building had before the completion (`set_state(2)` puts it in 2 after).
    pub cart: Option<u8>,
    /// Its stack unit (`+0x986`), which its owner's slot 0x1c0 makes in its place (the
    /// caller's, [`crate::world::World::stack_unit`]).
    pub stack: Option<StackUnit>,
    /// Its master disappears when built (`+0x984`): its destroy (slot 0xd0) ends the
    /// completion, after its technology (the caller's).
    pub vanishes: bool,
    /// Its construction started in the completion (`FUN_1411e5670` on a desolid one): its owner
    /// watches it (the caller's).
    pub started: bool,
    /// The completion's first part fills the pasture ([`FirstPart::pasture`], the caller's after
    /// the attributes).
    pub pasture: bool,
}

/// A completed building's stack unit (`FUN_1411dbb70`'s block for `+0x986`): the master its
/// owner's slot 0x1c0 makes (`FUN_14162ce00` with its flag set), the place, and the hit points
/// the building lacked (its master's less its own, which the new object loses).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StackUnit {
    /// The master (`+0x986`; master 1889 on mangrove or rice terrain, `+0xcd0`, makes 1893).
    pub master: i16,
    /// The building's place.
    pub at: [f32; 3],
    /// Its master's hit points (`+0x530`, a short) less its own.
    pub hp_lost: f32,
}

/// The completion's sprite (`FUN_1411dbb70` from state 0 to 2, after its storage): slot 0x68
/// (`FUN_141217000`) with the master's standing sprite (`+0x470`), then its sprite list's slot
/// 0x28 (`FUN_1413ba880`) sets that sprite's node to the frame its BUILD command chose
/// (`+0xc54`, [`crate::objects::unit::Unit::build_frame`]; slot 9 with it, [`ActiveSprite::set_frame`]).
pub(crate) fn show_built(
    sprite: &mut Option<crate::objects::sprite::ActiveSprite>,
    stats: Option<&mut crate::movement::MoveStats>,
    master: &Master,
    frame: u8,
    graphics: &BTreeMap<i16, crate::objects::sprite::Graphic>,
    sprite_rng: &mut crate::math::rng::Well512,
) {
    let standing = (0 <= master.standing_sprite)
        .then(|| graphics.get(&master.standing_sprite).copied())
        .flatten();
    if let Some(g) = standing {
        crate::objects::sprite::new_sprite(sprite, stats, g, sprite_rng);
        if let Some(s) = sprite.as_mut().filter(|s| s.graphic.id == g.id) {
            s.set_frame(i16::from(frame));
        }
    }
}

#[cfg(test)]
mod built_sprite_tests {
    use super::*;
    use crate::objects::sprite::{ActiveSprite, Graphic};

    /// The completion's sprite (`FUN_1411dbb70`): the foundation's construction sprite gives way
    /// to the master's standing one (slot 0x68), whose node takes the BUILD command's frame
    /// (slot 0x28, slot 9 with it); a static standing sprite takes the frame alone.
    #[test]
    fn a_completed_building_shows_its_standing_sprite_at_its_frame() {
        let g = |id, flags| Graphic {
            id,
            frames: 4,
            facets: 1,
            speed_multiplier: 1.0,
            frame_duration: 0.1,
            replay_delay: 0.0,
            flags,
        };
        let graphics = BTreeMap::from([(10, g(10, 1)), (11, g(11, 1))]);
        let master = Master {
            standing_sprite: 11,
            ..Master::default()
        };
        let mut rng = crate::math::rng::Well512::default();
        let mut sprite = Some(ActiveSprite::start(g(10, 1), &mut rng));
        show_built(&mut sprite, None, &master, 2, &graphics, &mut rng);
        let s = sprite.unwrap();
        assert_eq!((s.graphic.id, s.frame, s.last_frame), (11, 2, 2));
        let statics = BTreeMap::from([(11, g(11, 0))]);
        let mut none = None;
        show_built(&mut none, None, &master, 3, &statics, &mut rng);
        assert_eq!(none.map(|s| (s.graphic.id, s.frame)), Some((11, 3)));
    }
}

/// What a builder's Build reads as its update starts (`Unit::build_part`'s reads), besides
/// the action and the world it reaches through the step.
#[derive(Debug)]
pub(crate) struct BuildReads {
    /// The foundation (the action's target) when the builder's player holds it with its
    /// construction: the update reads and works it through the world.
    pub(crate) site: Option<i32>,
    /// The foundation's master, when the builder's player holds the foundation.
    pub(crate) built: Option<i16>,
    /// The foundation's `+0xcb0`: the object its completion's stack unit made
    /// ([`crate::objects::unit::Unit::stack_made`]).
    pub(crate) stack_made: Option<i32>,
    /// The state of its `MoveTo`'s target ([`BuildAction::walk_target`]), None when the world
    /// does not hold it.
    pub(crate) walk_target_state: Option<u8>,
    /// The state of each foundation its unit AI's queued build orders name (None for one that
    /// is gone).
    pub(crate) build_targets: Vec<(i32, Option<u8>)>,
    /// In state 8, what the drop-off reads of its second target ([`BuildAction::site`]): its
    /// state and [`crate::gathering::hunt::DropOff`]; None when the world does not hold it.
    pub(crate) deposit: Option<(u8, crate::gathering::hunt::DropOff)>,
    /// In state 8, the targets of its unit AI's queued orders 0x2c5, 0x2c9 and 0x2db the world
    /// no longer holds or whose seen state is above 5 (`FUN_140c5de60`).
    pub(crate) gone_orders: Vec<i32>,
    /// The main building that takes the work instead (`FUN_1411cc430`, [`WorkChain::main`]).
    pub(crate) main_work: Option<MainWork>,
    /// What the footprint clearing of its `set_state(7)` reads of the objects on the tiles
    /// ([`ClearLook`]) by id; none for one the world does not hold.
    pub(crate) looks: BTreeMap<i32, ClearLook>,
    /// The sprites its `MoveTo`s take when the task gives none.
    pub(crate) defaults: MoveDefaults,
    /// The builder's class (`+0x0a`, decoded).
    pub(crate) class: i16,
    /// Its foundation's master is a drop site for its civ (`FUN_140fde180`) and it carries a
    /// kind its player has: the end of the work gives its load to its player.
    pub(crate) drop_site: bool,
    /// Another object is the foundation's annex (its `+0xbc0` list), which its completion reads.
    pub(crate) annexed: bool,
    /// The foundation's annexes on the map, whose constructions the work starts first.
    pub(crate) annex_ids: Vec<i32>,
    /// The builder as the releaser of its targets (slot 0x180's argument).
    pub(crate) by: crate::objects::release::Releaser,
}

/// The fishing ships that work the fish traps they build (`FUN_1411945a0`: 13, 2356, 2171).
const FISHING_SHIPS: [i16; 3] = [13, 2356, 2171];

/// A second builder works at a third of the rate of the first (`DAT_142ed66b0`).
const EXTRA_BUILDER: f32 = crate::math::image::build::EXTRA_BUILDER;
/// The progress a started foundation holds before its first work (`0x2edbe6ff`, about 1e-10).
const STARTED: u32 = 0x2edb_e6ff;
/// The Wonder's master (`0x114`), whose construction rate the player's attribute raises at most to
/// `DAT_142ed66b4`.
const WONDER: i16 = 0x114;
const WONDER_RATE: f32 = crate::math::image::build::WONDER_RATE;
/// The castles and the wonders their owner built (attributes 173 and 174: their completion adds
/// 1, their death takes nothing), and the attribute their paid cost goes to rather than 98 and
/// 165 (184), all `FUN_1411dbb70`.
const CASTLES_BUILT: i16 = 173;
const WONDERS_BUILT: i16 = 174;
const CASTLE_WONDER_COST: i16 = 184;
/// The wall class (27, `0x1b`): two walls beside a builder on a footprint's diagonal stop it.
const WALL: i16 = 27;

/// The Build action's states the slice models.
const MOVE: u8 = 4;
const WORK: u8 = 6;
pub(crate) const PLACE: u8 = 7;
/// The walk to the action's point (`set_state(11)`).
const WALK_POINT: u8 = 11;
/// The wait for an occupied footprint (`set_state(222)`).
const WAIT: u8 = 222;
/// The waits a builder makes before it gives up (`0x15`).
const MAX_WAITS: u8 = 21;
/// 223 (`0xdf`): off the footprint, to a free tile beside it.
const CLEAR: u8 = 223;
/// 224 (`0xe0`): the work ended, the building completed.
pub(crate) const FINISHED: u8 = 224;
/// 1: the action is done.
const DONE: u8 = 1;
/// The last object state an action takes as its target (`FUN_140a7a230`: one above is refused).
const LAST_TAKEN: u8 = 6;
/// 3: the action waits without a target.
pub(crate) const IDLE: u8 = 3;
/// `set_state(2)`: the unit's stop halted the Build (slot 0x38); its next update fails it.
const HALTED: u8 = 2;
/// 13 (`0xd`): the Build failed; its next update tells its unit AI and halts it.
const FAILED: u8 = 13;
/// 8: the builder carries its load to a drop site (`set_state(8)`, [`BuildAction::set_drop`]).
pub const DROP: u8 = 8;
/// The drop-off's wait after a drop site out of reach (`0xc0400000`).
const DROP_WAIT: f32 = -3.0;
/// The drop-off reaches its drop site within this squared footprint gap (`DAT_14330eb8c`).
const DROP_REACH: f32 = crate::math::image::build::DROP_REACH;
/// The orders whose targets gone the unit AI drops after a drop-off (`FUN_140c5de60`: the bits
/// of `0x400011` over `order - 0x2c5`).
pub(crate) const DROPPED_ORDERS: [OrderKind; 3] =
    [OrderKind::Gather, OrderKind::Hunt, OrderKind::OnTarget];
/// The point a `MoveTo` without a target walks to (`FUN_140aa0df0` leaves its zeroed `+0x38`).
const NO_TARGET_POINT: [f32; 3] = [0.0; 3];
/// The buildings' master type (80).
const BUILDING_TYPE: u8 = 80;
/// The masters and the class the end of the work treats apart: 621 and 1889 among the
/// foundations, class 39 among the builders, 1893 and 1897 (`id - 0x765 & !4 == 0`), the fish
/// trap 199 and farms (class 49, `FUN_141194b20`).
const MASTER_621: i16 = 621;
pub(crate) const MASTER_1889: i16 = 1889;
/// The pasture master 1889's stack unit on mangrove or rice terrain (`0x765`).
pub(crate) const RICE_PASTURE: i16 = 0x765;
const CLASS_39: i16 = 39;
const FISH_TRAP: i16 = 199;
const FARM: i16 = 49;

#[cfg(test)]
mod stack_tests {
    use super::*;
    use crate::map::spatial::{Placed, Tiles};
    use crate::objects::lifecycle::{Class, Lifecycle};
    use crate::objects::masters::{BuildingMaster, Masters};
    use crate::objects::unit::Unit;
    use crate::world::{Player, World};

    /// A building master of `id`: class 3, radius `r`, height `z`, obstruction `kind`, train
    /// time `bt`, hit points `hp`, an annex of `head`, with `annexes` at their offsets.
    fn building(
        id: i16,
        (r, z, kind): (f32, f32, u8),
        (bt, hp): (i16, i16),
        head: i16,
        annexes: &[(i16, [f32; 2])],
    ) -> Master {
        let mut slots = [-1; 4];
        let mut offsets = [[0.0; 2]; 4];
        for (k, &(a, o)) in annexes.iter().enumerate() {
            slots[k] = a;
            offsets[k] = o;
        }
        Master {
            id,
            kind: 80,
            class: 3,
            radius: [r, r],
            height: z,
            tall: 0.0 < z,
            obstruction_type: kind,
            obstruction_class: 2,
            build_time: bt,
            trained_at: vec![-1],
            hit_points: hp,
            building: Some(BuildingMaster {
                stack_unit: -1,
                foundation_terrain: -1,
                tech: -1,
                annexes: slots,
                annex_offsets: offsets,
                head_unit: head,
                transform_unit: -1,
                ..BuildingMaster::default()
            }),
            ..Master::default()
        }
    }

    /// The town centre's masters as build 158041's `.dat` holds them for a civ without 2160: the
    /// foundation 621 (stack unit 109, disappears when built), the town centre 109 (annexes 618,
    /// 619, 620 and 890; its storage of population, 5 of mode 4, and of attribute 506, 1 of mode
    /// 16; a volley missile: the tower's unit AI), 618 (tall), 619 (its own annexes 1649 and
    /// 2160), 620, 1649 and 890 (class 27, hit points −1, tall).
    fn masters() -> Masters {
        let mut foundation = building(621, (2.0, 4.0, 2), (150, 2400), -1, &[]);
        if let Some(b) = foundation.building.as_mut() {
            b.stack_unit = 109;
            b.disappears_when_built = true;
        }
        let mut tc = building(
            109,
            (2.0, 0.0, 0),
            (100, 2400),
            621,
            &[
                (618, [1.0, -1.0]),
                (619, [-0.5, 0.5]),
                (620, [-1.0, 1.0]),
                (890, [-1.0, 1.0]),
            ],
        );
        tc.storage = vec![(4, 5.0, 4), (506, 1.0, 16), (-1, 0.0, 0)];
        tc.volley_missile = Some(54);
        let mut empty = building(890, (0.0, 4.0, 5), (-10, -1), 109, &[]);
        empty.class = 27;
        Masters::new([
            foundation,
            tc,
            building(618, (1.0, 3.0, 2), (100, 2400), 621, &[]),
            building(
                619,
                (1.5, 0.0, 0),
                (100, 2400),
                621,
                &[(1649, [0.75, -0.75]), (2160, [0.0, 0.0])],
            ),
            building(620, (1.0, 0.0, 0), (100, 2400), 621, &[]),
            building(1649, (0.0, 0.0, 0), (0, 1), 619, &[]),
            empty,
        ])
    }

    /// Player 1 with the town-centre foundation 8759 at (54, 147), built to its last 0.01
    /// seconds, its record on the map; the next id 8768.
    fn world() -> World {
        let ms = masters();
        let foundation = Unit {
            id: 8759,
            body: crate::objects::layers::Body::at([54.0, 147.0, 0.0]),
            life: Lifecycle {
                class: Class::Animated,
                state: 0,
                hp: 2399.0,
                master_id: 621,
                ..Lifecycle::default()
            },
            construction: Some(Construction {
                progress: 149.99,
                desolid: false,
                builders: 1,
                hit_points: 2400,
                build_time: 150,
                record: Obstruction { kind: 2, class: 2 },
                snow: false,
                sprite_override: false,
                started: false,
            }),
            pasture: -1,
            my_pasture: -1,
            ..Unit::default()
        };
        let mut t = World::new(
            0,
            1,
            0,
            vec![0; 2],
            vec![
                Player::default(),
                Player {
                    objects: vec![foundation].into(),
                    attributes: vec![0.0; 600],
                    ..Player::default()
                },
            ],
        );
        t.spatial = Spatial::with_tiles([200, 200], Tiles::new(200, 200, vec![0; 200 * 200]));
        t.spatial.terrain = Some(crate::map::terrain::Terrain::new(
            200,
            200,
            vec![(0, 255, -1); 200 * 200],
            vec![
                crate::map::terrain::TerrainKind {
                    shown: true,
                    flags: 0,
                    forest: false,
                };
                40
            ],
            vec![vec![0.0; 40]; 5],
        ));
        t.spatial.insert(
            8759,
            Placed {
                position: [54.0, 147.0, 0.0],
                radius: [2.0, 2.0],
                class: 3,
                player: 1,
                master: 621,
                obstruction_type: 2,
                buildable_on: 0,
                tall: true,
                speed: 0.0,
                unknown: None,
                obstruction: Some(Obstruction { kind: 2, class: 2 }),
            },
        );
        t.next_object_id = 8768;
        t.masters = vec![ms.clone(), ms];
        t
    }

    /// A completion's history event goes to its owner's history first, the owner's id as
    /// `data1` at the world's time (`FUN_1411dbb70`: `FUN_141560ae0` on `+0xdb8` with
    /// `(float)+0x718`, 0 and 0), then its attributes.
    #[test]
    fn a_castle_s_completion_records_its_owner_s_id_in_its_history() {
        let mut t = world();
        t.world_time = 912_000;
        t.players[1].history.add_entry(0, 3.0, 0.0);
        t.built(
            1,
            8759,
            Built {
                attributes: vec![(134, 1.0)],
                history: Some(crate::players::history::CASTLE_BUILT),
                ..Built::default()
            },
        )
        .unwrap();
        assert_eq!(
            t.players[1].history.events,
            [crate::players::history::Event {
                kind: crate::players::history::CASTLE_BUILT,
                time_slice: 1,
                world_time: 912_000,
                data1: 1.0,
                data2: 0.0,
                data3: 0.0,
            }]
        );
        assert_eq!(t.players[1].attributes[134], 1.0);
    }

    /// census v5's 273 stops "a completion of a stack unit or a vanishing building" (records/
    /// 428759682 update 2019): the work that completes the town-centre foundation 621 drops its
    /// record, its owner makes the stack unit 109 at its place with its flag (`FUN_14162ce00`):
    /// the town centre built already (its train time as build points, its hit points), its
    /// annexes depth first (618, 619 with its 1649, 620, 890; 2160 the player lacks), each at
    /// its offset and linked, the tall ones desolid; worked 10000 seconds at once it completes,
    /// its annexes first: its population (attribute 4) and its own store (506), less the hit
    /// points the foundation lacked; then 621 disappears (state 7, hit points 0) without paying
    /// attributes 98 and 165.
    #[test]
    fn a_town_centre_foundation_completes_into_the_town_centre_and_its_annexes() {
        let mut t = world();
        t.work_building(8759, 1.0).unwrap();
        let ids: Vec<(i32, i16, u8)> = t.players[1]
            .objects
            .iter()
            .map(|u| (u.id, u.life.master_id, u.life.state))
            .collect();
        assert_eq!(
            ids,
            [
                (8759, 621, 7),
                (8768, 109, 2),
                (8769, 618, 2),
                (8770, 619, 2),
                (8771, 1649, 2),
                (8772, 620, 2),
                (8773, 890, 2),
            ]
        );
        let at = |id: i32| t.spatial.get(id).unwrap().position;
        assert_eq!(
            [at(8769), at(8770), at(8771), at(8772)],
            [
                [55.0, 146.0, 0.0],
                [53.5, 147.5, 0.0],
                [54.25, 146.75, 0.0],
                [53.0, 148.0, 0.0]
            ]
        );
        let unit = |id: i32| t.players[1].objects.by_id(id).unwrap();
        assert_eq!(
            [8769, 8770, 8771, 8772, 8773].map(|a| unit(a).annex_of),
            [Some(8768), Some(8768), Some(8770), Some(8768), Some(8768)]
        );
        // the tower's unit AI for the town centre, the building's for the annexes
        assert_eq!(unit(8768).ai.as_ref().map(|ai| ai.ai_type), Some(15));
        assert_eq!(unit(8769).ai.as_ref().map(|ai| ai.ai_type), Some(17));
        // the foundation lacked 0.84 of its 2400 hit points after its last work: the town
        // centre lacks them too
        let lacked = 2400.0 - (2399.0 + (0.01_f32 * 2400.0) / 150.0);
        assert_eq!(
            (unit(8768).life.hp, unit(8768).store.clone()),
            (2400.0 - lacked, vec![(506, 1.0)])
        );
        let attributes = &t.players[1].attributes;
        assert_eq!(
            (attributes[4], attributes[98], attributes[165]),
            (5.0, 0.0, 0.0)
        );
        let gone = unit(8759);
        assert_eq!((gone.life.hp, gone.roo_called), (0.0, true));
        assert_eq!(t.spatial.get(8759).unwrap().obstruction, None);
    }

    /// A new building's record (`FUN_14118c540` from its place): a master without a height (a
    /// pasture's post, master 1888: collision height 0) gets none, though its obstruction kind
    /// and class would give one; with a height it does (431153923's builder, update 412).
    #[test]
    fn a_new_building_without_a_height_gets_no_record() {
        for (height, record) in [(0.0, None), (1.0, Some((5, 2)))] {
            let mut t = world();
            for ms in &mut t.masters {
                let m = ms.get_mut(109).unwrap();
                m.build_time = 0;
                m.obstruction_type = 5;
                m.obstruction_class = 2;
                m.height = height;
                m.building.as_mut().unwrap().annexes = [-1; 4];
            }
            let (id, _) = t
                .make_building(1, 109, Ok(None), [80.0, 80.0, 0.0])
                .unwrap();
            let placed = t.spatial.get(id).unwrap().obstruction;
            assert_eq!(placed.map(|o| (o.kind, o.class)), record, "height {height}");
        }
    }

    /// A gate foundation's stack gate takes the foundation's lock (`FUN_1411dbb70`: the new
    /// gate's slot 0x670 with the old one's `+0xc7c`): locked, the new gate's last move time is
    /// the world time plus 1; unlocked it stays unlocked and keeps its last move time.
    #[test]
    fn a_gate_s_stack_gate_takes_the_foundation_s_lock() {
        for locked in [true, false] {
            let mut t = world();
            t.world_time = 5000;
            for ms in &mut t.masters {
                for id in [621, 109] {
                    let m = ms.get_mut(id).unwrap();
                    m.class = 39;
                    m.storage.clear();
                    m.volley_missile = None;
                    // every building holds its movement state
                    m.moving = Some(crate::movement::MoveStats {
                        fly_mode: 0,
                        speed: 0.0,
                        base: 0.0,
                        walk_speed: 0.0,
                        run_speed: 0.0,
                        stand_speed: 0.0,
                        turn_rate: 0.0,
                        turn_radius: 0.0,
                        turn_radius_speed: f32::MAX,
                        radius: [2.0, 2.0, 0.0],
                        restriction: 0,
                        restriction_mask: 255,
                        turn_time: 0.0,
                        move_algorithm: 0,
                        min_collision: 0.0,
                        fast_turn: false,
                        facets: None,
                        weapon_range: 0.0,
                    });
                    if let Some(b) = m.building.as_mut() {
                        b.annexes = [-1; 4];
                    }
                }
            }
            t.players[1].objects[0].gate = Some(crate::world::Gate {
                locked,
                ..crate::world::Gate::default()
            });
            t.stack_unit(
                1,
                8759,
                StackUnit {
                    master: 109,
                    at: [54.0, 147.0, 0.0],
                    hp_lost: 0.0,
                },
            )
            .unwrap();
            let gate = t.players[1].objects.by_id(8768).unwrap();
            assert_eq!(gate.gate.map(|g| g.locked), Some(locked));
            let last_move = gate.body.mover().map(|m| m.last_move);
            assert_eq!(
                last_move,
                Some(if locked { 5001 } else { 5000 }),
                "{locked}"
            );
        }
    }

    /// The stack unit's completion tells each attacking unit of the foundation (`+0xce0`) the
    /// world still holds of the new object (slot 0x140, not modelled); one it no longer holds
    /// is passed over.
    #[test]
    fn a_stack_unit_s_completion_reads_the_foundation_s_attacking_units() {
        let mut gone = world();
        gone.players[1].objects[0].attacking_units = vec![99_999];
        assert_eq!(gone.work_building(8759, 1.0), Ok(()));
        let mut held = world();
        let first = held.players[1].objects[0].id;
        held.players[1].objects[0].attacking_units = vec![first];
        assert_eq!(
            held.work_building(8759, 1.0),
            Err(GenieError::unsupported(
                "a stack unit's completion with other players' units on it (+0xce0)"
            ))
        );
    }
}

#[cfg(test)]
mod step_tests;
#[cfg(test)]
mod tests;
