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

use crate::lifecycle::Lifecycle;
use crate::masters::Master;
use crate::move_to::{Goal, start_walk, walk_on};
pub use crate::move_to::{MoveTo, Walk};
use crate::movement::footprint_gap;
use crate::order::Place;
use crate::pathing::Pathing;
use crate::spatial::{Obstruction, Spatial};
use crate::turn::{CASTLE, CASTLES, MONASTERIES, MONASTERY, WONDERS};
use crate::unit_ai::{GenieError, UnitAi};

/// The Build action's type, which word 9 adds.
pub const BUILD: i16 = 101;

/// The gap below which a `MoveTo` has arrived (`DAT_142eb99cc`).
const ARRIVED: f32 = crate::image::build::ARRIVED;
/// The villager's class (4, `-0xea8` encoded): Build's walk off the footprint returns at once.
const VILLAGER: i16 = 4;

/// The Build action of a builder (`TRIBE_Action_Build`, type 101).
#[derive(Clone, Copy, Debug, PartialEq)]
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
}

/// What a builder's Build task gives its work: the task's work range (`+0x30`; its `MoveTo` to the
/// foundation stops this near) and work value (`+0x28`, `work_val1`), and the builder's work rate:
/// its master's `work_rate` (`+0x798`) plus its own `mWorkRatePowerup` (`+0xa30`, slot 0x6e8).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BuildTask {
    /// The task's work range.
    pub range: f32,
    /// The task's work value.
    pub work_value: f32,
    /// The builder's work rate: its master's `work_rate` (`+0x798`) plus its `mWorkRatePowerup`
    /// (`+0xa30`).
    pub work_rate: f32,
}

/// A building under construction (`TRIBE_Building_Object`), with what its master says of it.
#[derive(Clone, Copy, Debug, PartialEq)]
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
    pub snow: bool,
    /// `+0x788`, `sprite_override_id`, is not −1.
    pub sprite_override: bool,
    /// Its construction started in this update: its owner's watched list takes it
    /// (`FUN_1411e5670`'s `FUN_141090430`, the caller's).
    pub started: bool,
}

/// What a builder's work reaches of its foundation.
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
    /// The builder count (`+0x9e8`) of its main building when it is a building annex
    /// (`+0xbb8`): the building `FUN_140aaddb0` gives counts the builders instead
    /// ([`Site::builders`]).
    pub main_builders: Option<&'a mut u8>,
}

impl<'a> Site<'a> {
    /// The builder count of the building `FUN_140aaddb0` gives a Build on this foundation: its
    /// main building's for a building annex, else its own (`+0x9e8`).
    pub fn builders(&mut self) -> &mut u8 {
        match self.main_builders.as_deref_mut() {
            Some(b) => b,
            None => &mut self.construction.builders,
        }
    }

    /// [`Site::builders`] for the rest of the site's borrow.
    #[must_use]
    pub fn into_builders(self) -> &'a mut u8 {
        match self.main_builders {
            Some(b) => b,
            None => &mut self.construction.builders,
        }
    }
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
/// ([`crate::turn::Turns::work_building`] on the annex, [`Worked::Main`]).
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
    /// The game's setting at `+0x680` ([`crate::turn::Turns::mode_680`]).
    pub mode_680: i32,
    /// The owner's attributes 237, 238 and 239 (`player+0x768`..`+0x778`): a farm's bonus when
    /// built, its amount, its resource and a master near which it is given
    /// ([`Completion::first_part`]).
    pub farm_bonus: [f32; 3],
    /// The owner's masters, whose table `FUN_141659d20` reads.
    pub masters: Option<&'a crate::masters::Masters>,
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
    /// search for resources follows ([`crate::turn::Turns::cart_search`]).
    pub cart: bool,
}

impl Completion<'_> {
    /// The completion's first part (`FUN_1411c6bd0`), which the building's `set_state` runs from
    /// state 0 to any state of 2 or more: a farm (class 49) whose build points reached its build
    /// time (COMISS, JC: a NaN does not) gets its owner's bonus when attributes 238 (a resource)
    /// and 239 (a master), cut to integers (CVTTSS2SI), are both 0 or more (`FUN_1411c66a0`): a
    /// master 239 the owner has (`FUN_141659d20`) gives it only near the farm
    /// (`FUN_1411d3cd0`, not modelled); a resource up to the owner's attribute count gets
    /// attribute 237's amount (slot 0xf8 with 1 and 1); a pasture (1893, 1897) then fills
    /// (`FUN_1411c8410`, not modelled); a master with a task of action type 0x99 and a unit AI
    /// runs its cart's search (the tail's game mode 4 test, `FUN_140c87f20`, holds in a game).
    /// Not modelled: a computer player's (`FUN_1411e7ca0`) and an overridden sprite's master
    /// copy (`+0x788`). Not held: the owner's `+0x137c` (cleared for master 109), the object
    /// tracker (`FUN_14104eac0`, `FUN_14104faf0`) and the farm's `+0xcac`.
    ///
    /// # Errors
    ///
    /// What it does not model.
    pub fn first_part(&self, c: &Construction) -> Result<FirstPart, GenieError> {
        use crate::checkpoint::cvttss2si;
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
            if matches!(master.id, 1893 | 1897) {
                return Err(GenieError::unsupported(
                    "a pasture's completion (FUN_1411c8410)",
                ));
            }
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
    /// caller's, [`crate::turn::Turns::stack_unit`]).
    pub stack: Option<StackUnit>,
    /// Its master disappears when built (`+0x984`): its destroy (slot 0xd0) ends the
    /// completion, after its technology (the caller's).
    pub vanishes: bool,
    /// Its construction started in the completion (`FUN_1411e5670` on a desolid one): its owner
    /// watches it (the caller's).
    pub started: bool,
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

/// What a builder brings to its Build action's update besides its walk.
#[derive(Debug)]
pub struct Builder<'a> {
    /// The builder's class (`+0x0a`, decoded).
    pub class: i16,
    /// Its master's id.
    pub master: i16,
    /// What it carries (`+0x310`).
    pub held: f32,
    /// The kind of what it carries (`+0x308`, −1 for none).
    pub held_kind: i16,
    /// Its foundation's master is a drop site for its civ (`FUN_140fde180`): the end of the work
    /// gives its load to its player.
    pub drop_site: bool,
    /// Its unit AI, which a finished Build hands its queued orders to.
    pub ai: Option<&'a mut UnitAi>,
    /// `+0x889`: the builder's waits for an occupied footprint (not saved, 0 at a load).
    pub waits: &'a mut u8,
    /// The state of each foundation its unit AI's queued build orders name (None for one that
    /// is gone), as the update starts.
    pub build_targets: &'a [(i32, Option<u8>)],
    /// What the footprint clearing of its `set_state(7)` reads of the objects on the tiles
    /// ([`ClearLook`]), as the update starts; None for one the caller does not hold.
    pub look: Looks<'a>,
    /// The state of its `MoveTo`'s target ([`BuildAction::walk_target`]) as the update starts,
    /// None when the world does not hold it.
    pub walk_target_state: Option<u8>,
}

/// What the footprint clearing reads of the objects on the tiles, by id ([`ClearLook`]).
#[derive(Clone, Copy)]
pub struct Looks<'a>(pub &'a dyn Fn(i32) -> Option<ClearLook>);

impl std::fmt::Debug for Looks<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Looks")
    }
}

/// What the end of a Build's work leads to (case 0xdf).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AfterWork {
    /// The Build ends (`set_state(1)`; the update returns 1).
    Done,
    /// The builder looks for more work around the building.
    Search,
    /// The builder works the building (slot 0x7e8).
    WorksIt,
}

/// The fishing ships that work the fish traps they build (`FUN_1411945a0`: 13, 2356, 2171).
const FISHING_SHIPS: [i16; 3] = [13, 2356, 2171];

/// What a Build action's update leaves to its unit and player.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BuildStep {
    /// The update returned 1: the action list removes the action, whose destructor
    /// (`FUN_140aadbf0`) releases the foundation (slot 0x180).
    pub done: bool,
    /// The builder works the building it built (a villager its farm, a fishing ship its fish
    /// trap: slot 0x7e8 on it, the caller's).
    pub works_it: bool,
    /// The work completed the building: what its owner gains.
    pub completed: Option<Built>,
    /// The action let its foundation go (slot 0x68 with none, `FUN_140ab0300` ->
    /// `FUN_140a7a230`: the foundation's slot 0x180): the caller's.
    pub released: bool,
    /// Its `set_state(3)` found no orders in its unit AI's queue and posted notify 0x202 (with
    /// the build action's value) to it: the caller's.
    pub notified: bool,
    /// Its failure (the update's case 0xc) posts notify 0x1f9 (with the build action's value)
    /// to its unit AI: the caller's.
    pub failed: bool,
    /// The unit takes its master's standing sprite (slot 0x68 in `set_state` 1, 2, 3 and 13):
    /// the caller's.
    pub stands: bool,
    /// The work ended and the builder looks for more work around the building (`FUN_1415cc4e0`,
    /// [`crate::gather_search::more_work`]): the caller's, which works what it finds, else lets
    /// the foundation go ([`BuildAction::no_more_work`], then as [`BuildStep::released`]).
    pub search: bool,
    /// The units on the foundation's footprint besides the builder, which its test
    /// (`FUN_14123bb20`) would send off: the caller's.
    pub occupants: Vec<i32>,
    /// The objects the footprint clearing of its `set_state(7)` deletes (`FUN_1415c3620` with
    /// `param_6`), in order; their records went already: the caller's.
    pub cleared: Vec<Cleared>,
    /// The end of the work at a drop site (`set_state(224)`, `FUN_140aadc50`): the load the
    /// builder gives its player (kind, amount; the food kinds 15 to 17 as 0), which it no longer
    /// carries (`FUN_14119d310` with −1 and 0): the caller's.
    pub dropped: Option<(i16, f32)>,
    /// The seconds of work an annex's main building takes ([`Worked::Main`]): the caller's
    /// after the update ([`crate::turn::Turns::work_building`] on the foundation).
    pub main_work: Option<f32>,
}

/// A load a builder drops at the end of its work: kind and amount ([`BuildStep::dropped`]).
type Dropped = Option<(i16, f32)>;

/// What a builder carries when its work ends, and whether its foundation takes it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Load {
    amount: f32,
    kind: i16,
    drop_site: bool,
}

/// A second builder works at a third of the rate of the first (`DAT_142ed66b0`).
const EXTRA_BUILDER: f32 = crate::image::build::EXTRA_BUILDER;
/// The progress a started foundation holds before its first work (`0x2edbe6ff`, about 1e-10).
const STARTED: u32 = 0x2edb_e6ff;
/// The Wonder's master (`0x114`), whose construction rate the player's attribute raises at most to
/// `DAT_142ed66b4`.
const WONDER: i16 = 0x114;
const WONDER_RATE: f32 = crate::image::build::WONDER_RATE;
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
const PLACE: u8 = 7;
/// The walk to the action's point (`set_state(11)`).
const WALK_POINT: u8 = 11;
/// The wait for an occupied footprint (`set_state(222)`).
const WAIT: u8 = 222;
/// The waits a builder makes before it gives up (`0x15`).
const MAX_WAITS: u8 = 21;
/// 223 (`0xdf`): off the footprint, to a free tile beside it.
const CLEAR: u8 = 223;
/// 224 (`0xe0`): the work ended, the building completed.
const FINISHED: u8 = 224;
/// 1: the action is done.
const DONE: u8 = 1;
/// 3: the action waits without a target.
const IDLE: u8 = 3;
/// `set_state(2)`: the unit's stop halted the Build (slot 0x38); its next update fails it.
const HALTED: u8 = 2;
/// 13 (`0xd`): the Build failed; its next update tells its unit AI and halts it.
const FAILED: u8 = 13;
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
/// The build order (`0x2be`), which the end of the work drops from the AI's queue once its
/// foundation is gone or built (`FUN_140c5dcd0`).
const BUILD_ORDER: i32 = crate::ai_kinds::ORDER_BUILD;

impl BuildAction {
    /// The action's start (slot 5, `0x140aadd80`) on the foundation `target` at `at`: state 4
    /// for a target, whose `set_state` (`0x140aae200`) takes the foundation's place as the point
    /// and creates the `MoveTo` to it; the `MoveTo` starts arrived if the footprints touch
    /// (`gap`).
    #[must_use]
    pub fn start(target: i32, at: [f32; 3], gap: f32, task: BuildTask) -> Self {
        Self {
            target,
            state: MOVE,
            move_to: Some(if gap > ARRIVED {
                MoveTo::Start
            } else {
                MoveTo::Arrived
            }),
            goal: None,
            walk_target: target,
            point: at,
            task,
            added: false,
            timer: 0.0,
        }
    }

    /// One update (slot 6, `0x140ab0370`), which first updates the `MoveTo` (`0x140aa2a70`); a
    /// moving `MoveTo` calls the motion step (object slot 193). `site` is the foundation, when it
    /// is the builder's player's. Before its state's case it checks its foundation: one that is
    /// no longer under construction (a state above 0) ends the work (`set_state(224)`) unless the
    /// Build already ended (224, 4, 8); one that is gone, a dead one, master 621, a builder of
    /// class 39 and master 1889 of a human player are not modelled.
    ///
    /// # Panics
    ///
    /// Never: a builder clearing the footprint (state 223) always has the point its `set_state`
    /// gave it.
    pub fn update(
        &mut self,
        mut walk: Walk<'_>,
        site: Option<Site<'_>>,
        builder: Builder<'_>,
    ) -> Result<BuildStep, GenieError> {
        let mut step = BuildStep::default();
        // state 3 without a target: the update does nothing (0x140ab0370's default case)
        if self.state == IDLE && self.target == -1 {
            return Ok(step);
        }
        let computer = walk.plan.as_ref().is_some_and(|p| p.computer);
        let load = Load {
            amount: builder.held,
            kind: builder.held_kind,
            drop_site: builder.drop_site,
        };
        let mut site = site;
        // the target's id names no object of the world (FUN_1415417a0): slot 0x68 with none (its
        // builders' count went with it); then a Build in state 2 or 8 goes on, any other waits
        // (set_state 3) and the update returns 0
        if self.target != -1 && site.is_none() && walk.world.get(self.target).is_none() {
            self.added = false;
            self.target = -1;
            if !matches!(self.state, 2 | 8) {
                self.idle(&mut walk, None, builder, &mut step)?;
                return Ok(step);
            }
        }
        let mut walked_from = None;
        if let Some(site) = site.as_mut() {
            let master = site.completion.master;
            let state = site.life.state;
            if !computer
                && master.is_some_and(|m| m.id == MASTER_1889)
                && builder.ai.is_some()
                && state != 0
            {
                return Err(GenieError::unsupported("a Build on master 1889"));
            }
            if state != 0 && !matches!(self.state, FINISHED | MOVE | 8) {
                // 621 and the gates (class 39) end the work once built, else walk back
                let special = master.is_some_and(|m| m.id == MASTER_621 || m.class == CLASS_39);
                let c = &*site.construction;
                let unbuilt = c.progress < f32::from(c.build_time);
                if (special || state < 3) && !(special && 2 < state && unbuilt) {
                    step.dropped = self.finish(&mut walk, Some(site.builders()), load)?;
                } else {
                    // set_state(4) toward it, then slot 0x68 with none: the caller releases it
                    walked_from = Some(self.target);
                    self.restart(&mut walk, Some(site.builders()))?;
                    self.target = -1;
                    step.released = true;
                }
            }
        }
        if self.state == FINISHED {
            match self.after_work(&mut walk, site.as_ref(), computer, builder, &mut step)? {
                AfterWork::Done => step.done = true,
                AfterWork::Search => step.search = true,
                AfterWork::WorksIt => step.works_it = true,
            }
            return Ok(step);
        }
        let mut walk = walk;
        // the MoveTo's update first (`0x140aa2a70` through `FUN_140a992b0`, states 4 and 223):
        // its target in a state above 6 lets it go (slot 0x68 with none) and the quiet MoveTo
        // returns 5. A target the world removed is no longer the MoveTo's: its removal let the
        // MoveTo go (slot 0x180's release clears the pointer and the id), which walks on.
        if self.walk_target != -1 && builder.walk_target_state.is_none() {
            self.walk_target = -1;
        }
        if matches!(self.state, MOVE | CLEAR)
            && self.move_to.is_some()
            && self.walk_target != -1
            && builder.walk_target_state.is_some_and(|s| 6 < s)
        {
            self.walk_target = -1;
            self.lose_foundation(&mut walk, site, &mut step)?;
            return Ok(step);
        }
        let r = match (self.state, self.move_to) {
            // set_state(6) without a target: set_state(3)
            (4, Some(MoveTo::Arrived)) if self.target == -1 => {
                return self
                    .idle(&mut walk, site, builder, &mut step)
                    .map(|()| step);
            }
            // set_state(6): the prelude's MoveTo teardown stops the unit (slot 0x5d0)
            (4, Some(MoveTo::Arrived)) => {
                self.leave(&mut walk, None)?;
                self.state = WORK;
                Ok(())
            }
            // the MoveTo set_state(4) made walks to the foundation it let go
            (4, Some(MoveTo::Start)) => {
                self.move_to = Some(MoveTo::Moving);
                let range = self.task.range;
                let to = walked_from.unwrap_or(self.target);
                self.move_to = Some(start_walk(walk, Goal::Target(to), range, false)?);
                Ok(())
            }
            (4 | CLEAR | WALK_POINT, Some(MoveTo::Moving)) => {
                self.move_to = Some(walk_on(walk)?);
                Ok(())
            }
            (4, None) => Err(GenieError::unsupported(
                "a Build action in state 4 without its MoveTo",
            )),
            // case 0: the update returns 1, the action list removes it
            (DONE, _) => {
                step.done = true;
                Ok(())
            }
            // case 1: set_state(13); the update returns 0
            (HALTED, _) => self.end(&mut walk, site.map(Site::into_builders), FAILED, &mut step),
            // case 0xc: the unit's slot 0x328 posts notify 0x1f9 (the Build's action value, 0, 0)
            // to its unit AI; set_state(2) ([`BuildAction::halt`]: out of the builders, the
            // MoveTo's teardown, the unit's standing sprite, the target let go) and the update
            // returns 3: the action list removes the Build (FUN_140a98e20), as for 1
            (FAILED, _) => {
                step.failed = true;
                let counted = self.added;
                let (_, target) = self.halt(walk.mover.as_deref_mut())?;
                if counted && let Some(b) = site.map(Site::into_builders) {
                    *b = b.wrapping_sub(1);
                }
                step.stands = true;
                step.released = target != -1;
                step.done = true;
                Ok(())
            }
            (WORK, _) => {
                let (waits, look) = (builder.waits, builder.look);
                // case 5: FUN_141162db0 turns the builder to face its foundation
                let at = walk
                    .world
                    .get(walk.me.id)
                    .map_or(walk.me.position, |p| p.position);
                let to = walk
                    .world
                    .get_known(self.target)?
                    .ok_or(GenieError::unsupported(
                        "a builder's foundation without a place",
                    ))?
                    .position;
                let mover = walk
                    .mover
                    .as_deref_mut()
                    .ok_or(GenieError::unsupported("a builder without movement state"))?;
                let now = walk.plan.as_ref().map_or(0, |p| p.now);
                if !crate::attack::face_target(mover, at, to, now)? {
                    return Err(GenieError::unsupported(
                        "a builder that turns over time to face its foundation",
                    ));
                }
                self.place(&mut walk, site, (waits, look), &mut step)
            }
            // case 0xdd: the wait runs down by the world's dt, then set_state(7) again
            (WAIT, _) => {
                self.timer -= walk.dt;
                if 0.0 < self.timer {
                    return Ok(step);
                }
                self.timer = 0.0;
                self.place(&mut walk, site, (builder.waits, builder.look), &mut step)
            }
            (PLACE, _) => self.work(walk, site, load).map(|(c, d, m)| {
                step.completed = c;
                step.dropped = d;
                step.main_work = m;
            }),
            // the quiet MoveTo to a point of set_state(223) and set_state(11)
            (CLEAR | WALK_POINT, Some(MoveTo::Start)) => {
                self.move_to = Some(MoveTo::Moving);
                let goal = self
                    .goal
                    .expect("set_state(223) and set_state(11) set the goal");
                self.move_to = Some(start_walk(walk, Goal::Point(goal), 0.0, true)?);
                Ok(())
            }
            // case 0xde: the MoveTo ended (its update in state 1 returns 1): a villager goes
            // back to the foundation (set_state 4)
            (CLEAR, Some(MoveTo::Arrived)) => self.back(&walk),
            (CLEAR, _) => Err(GenieError::unsupported(
                "a builder off its footprint without its MoveTo",
            )),
            // case 10, the walk to the point: the MoveTo at it (1), failed (3) or gone (the list
            // empty, 1) makes the Build wait (set_state(3)); its target gone (4) cannot be, as
            // it has none
            (WALK_POINT, Some(MoveTo::Arrived | MoveTo::Failed) | None) => {
                return self
                    .idle(&mut walk, site, builder, &mut step)
                    .map(|()| step);
            }
            (WALK_POINT, Some(MoveTo::Waiting)) => Err(GenieError::unsupported(
                "a Build's walk to its point that waits (MoveTo state 2)",
            )),
            // the default case (states 0, 3, 5, 8 to 10, 12 and 14 to 221): the update returns 0
            _ => Ok(()),
        };
        r.map(|()| step)
    }

    /// The prelude of every `set_state` (`0x140aae200`): a builder counted among the builders
    /// of `FUN_140aaddb0`'s building (`builders`, [`Site::builders`]: none once its foundation
    /// is gone) leaves them, and the sub-actions go (`FUN_140a98620`: a `MoveTo`'s teardown
    /// stops the unit, slot 0x5d0).
    fn leave(&mut self, walk: &mut Walk<'_>, builders: Option<&mut u8>) -> Result<(), GenieError> {
        if self.added {
            if let Some(b) = builders {
                *b = b.wrapping_sub(1);
            }
            self.added = false;
        }
        if self.move_to.take().is_some()
            && let Some(m) = walk.mover.as_deref_mut()
        {
            m.stop()?;
        }
        self.goal = None;
        self.walk_target = -1;
        Ok(())
    }

    /// `set_state(224)` (`0x140aae200` case 0xe0): the work ended. A builder carrying something
    /// (`+0x310` above 0, its kind not −1) on a drop site of its civ (`FUN_140fde180`) drops it
    /// (`FUN_140aadc50`): returns the load its player gets ([`BuildStep::dropped`]).
    fn finish(
        &mut self,
        walk: &mut Walk<'_>,
        builders: Option<&mut u8>,
        load: Load,
    ) -> Result<Dropped, GenieError> {
        if self.added && builders.is_none() {
            return Err(GenieError::unsupported(
                "the end of a Build counted among another player's builders",
            ));
        }
        self.leave(walk, builders)?;
        self.state = FINISHED;
        if !(load.drop_site && 0.0 < load.amount && 0 <= load.kind) {
            return Ok(None);
        }
        // FUN_140aadc50: the food kinds become food
        let kind = if matches!(load.kind, 15..=17) {
            0
        } else {
            load.kind
        };
        Ok(Some((kind, load.amount)))
    }

    /// Case 0xdf of the update, the work ended (224): a farm, the fish trap and masters 1893 and
    /// 1897 are not modelled; the builder's unit AI drops its build orders on foundations gone or
    /// built (`FUN_140c5dcd0`); with orders left in its queue (`FUN_140c564a0`) it has no order
    /// and no action (`FUN_140c64b60`, `FUN_140c64b50` with −1) and the Build ends (`set_state(1)`,
    /// then the update returns 1: Some(true)). A builder without queued orders looks for more
    /// work around the building (`FUN_1415cc4e0`, Some(false): [`BuildStep::search`], the
    /// caller's). A computer player's builder is not modelled.
    fn after_work(
        &mut self,
        walk: &mut Walk<'_>,
        site: Option<&Site<'_>>,
        computer: bool,
        builder: Builder<'_>,
        step: &mut BuildStep,
    ) -> Result<AfterWork, GenieError> {
        if computer {
            return Err(GenieError::unsupported(
                "a computer player's builder after its work",
            ));
        }
        let master = site
            .and_then(|s| s.completion.master)
            .ok_or(GenieError::unsupported(
                "the end of a Build on a foundation the slice does not read",
            ))?;
        // LAB_140ab09af: a villager works the farm it built (FUN_141194b20, FUN_141195750), a
        // fishing ship its fish trap (FUN_141194640, FUN_1411945a0): its unit AI drops its build
        // orders (FUN_140c5e020 with 0x2be), then slot 0x7e8 on the building (the caller's)
        let farm = master.class == FARM && master.id != FISH_TRAP;
        let works_it = (farm && builder.class == VILLAGER)
            || (master.id == FISH_TRAP && FISHING_SHIPS.contains(&builder.master));
        if matches!(master.id, 1893 | 1897) {
            return Err(GenieError::unsupported(
                "the end of a Build on a pasture (its animals, slot 0x780)",
            ));
        }
        let ai = builder.ai.ok_or(GenieError::unsupported(
            "a builder without a unit AI after its work",
        ))?;
        if works_it {
            ai.orders.retain(|o| o.order != BUILD_ORDER);
            return Ok(AfterWork::WorksIt);
        }
        // FUN_140c5dcd0 (the building just completed is in state 2 now)
        drop_done_orders(
            ai,
            site.map(|s| (s.id, s.life.state)),
            builder.build_targets,
        );
        if ai.orders.is_empty() {
            return Ok(AfterWork::Search);
        }
        ai.end_order_and_action()?;
        self.end(walk, None, DONE, step)?;
        Ok(AfterWork::Done)
    }

    /// `set_state(4)` (`0x140aae200` case 4) toward its foundation `self.target`: the prelude
    /// (it leaves the foundation's builders, its `MoveTo` goes: the unit stops), then a new
    /// `MoveTo` to the foundation (`FUN_140aa0df0`), arrived when the footprints touch; its point
    /// (`+0x38`..`+0x40`, the foundation's place) is not held.
    ///
    /// # Errors
    ///
    /// A builder without movement state, a foundation without a place.
    fn restart(
        &mut self,
        walk: &mut Walk<'_>,
        builders: Option<&mut u8>,
    ) -> Result<(), GenieError> {
        self.leave(walk, builders)?;
        let mover = walk
            .mover
            .as_deref()
            .ok_or(GenieError::unsupported("a builder without movement state"))?;
        let at = walk
            .world
            .get_known(self.target)?
            .ok_or(GenieError::unsupported("a foundation that is gone"))?;
        let ground = [mover.stats.radius[0], mover.stats.radius[1]];
        let gap = footprint_gap(walk.me.position, ground, at.position, at.radius);
        *self = Self::start(self.target, at.position, gap, self.task);
        Ok(())
    }

    /// `set_state(3)` (`0x140aae200` case 3): the prelude (out of the foundation's builders, the
    /// `MoveTo`'s teardown), then with a unit AI its build orders on foundations gone or no
    /// longer in state 0 leave its queue (`FUN_140c5dcd0`); with orders left (`FUN_140c564a0`)
    /// it has no order and no action (`FUN_140c64b60`, `FUN_140c64b50` with −1) and the Build
    /// ends (`set_state(1)`: the update after removes it); with none it posts notify 0x202 to
    /// its unit AI ([`BuildStep::notified`]) and takes its standing sprite (slot 0x68,
    /// [`BuildStep::stands`]).
    ///
    /// # Errors
    ///
    /// A unit AI on a patrol order.
    fn idle(
        &mut self,
        walk: &mut Walk<'_>,
        site: Option<Site<'_>>,
        builder: Builder<'_>,
        step: &mut BuildStep,
    ) -> Result<(), GenieError> {
        let built = site.as_ref().map(|s| (s.id, s.life.state));
        self.leave(walk, site.map(Site::into_builders))?;
        self.state = IDLE;
        let mut left = 0;
        if let Some(ai) = builder.ai {
            drop_done_orders(ai, built, builder.build_targets);
            if !ai.orders.is_empty() {
                ai.end_order_and_action()?;
                self.end(walk, None, DONE, step)?;
            }
            left = ai.orders.len();
        }
        if left == 0 {
            step.notified = true;
            step.stands = true;
        }
        Ok(())
    }

    /// Slot 0x48 (`FUN_140ab1b30`), the work command's target for the Build's own task, here its
    /// own foundation again (slot 0x68 with the same target does nothing): working (state 6) it
    /// stays, as does a computer player's builder walking to it (state 4); else `set_state(4)`:
    /// it leaves the builders of `FUN_140aaddb0`'s building (`builders`, an annex's main
    /// building's) and its `MoveTo` (the unit stops, slot 0x5d0), and a new `MoveTo` to the
    /// foundation starts, arrived when the footprints touch (`gap`).
    ///
    /// # Errors
    ///
    /// A builder counted among the builders of a foundation the caller does not hold.
    pub fn retake(
        &mut self,
        (at, gap): ([f32; 3], f32),
        mover: Option<&mut crate::movement::Mover>,
        builders: Option<&mut u8>,
        computer: bool,
    ) -> Result<(), GenieError> {
        if self.state == WORK || (self.state == MOVE && computer) {
            return Ok(());
        }
        if self.added {
            let b = builders.ok_or(GenieError::unsupported(
                "a builder retaking a foundation the slice does not hold",
            ))?;
            *b = b.wrapping_sub(1);
        }
        if self.move_to.is_some()
            && let Some(m) = mover
        {
            m.stop()?;
        }
        *self = Self::start(self.target, at, gap, self.task);
        Ok(())
    }

    /// Slot 0x48 (`FUN_140ab1b30`) on another foundation of the builder's player: slot 0x68
    /// (`FUN_140ab0300`) takes it (the caller's builder count, release and take; the Build is no
    /// longer among the builders) and `set_state(4)` starts over: its `MoveTo` goes (the unit
    /// stops, slot 0x5d0) and a new one to the foundation starts, arrived when the footprints
    /// touch (`gap`).
    ///
    /// # Errors
    ///
    /// The unit's stop that the motion step does not model.
    pub fn retarget(
        &mut self,
        (target, at): (i32, [f32; 3]),
        gap: f32,
        mover: Option<&mut crate::movement::Mover>,
    ) -> Result<(), GenieError> {
        if self.move_to.is_some()
            && let Some(m) = mover
        {
            m.stop()?;
        }
        *self = Self::start(target, at, gap, self.task);
        Ok(())
    }

    /// Slot 0x38 (`FUN_140ab0350`) as the unit's stop calls it: `set_state(2)` (`0x140aae200`):
    /// a counted builder leaves its foundation's builders first (the caller's,
    /// [`crate::turn::Turns::leave_builders`]: true when it counted), the sub-actions go (a
    /// `MoveTo`'s teardown stops the unit), the unit takes its standing sprite (slot 0x68 with the
    /// master's `+0x470`, the caller's), the foundation and the second target go (slot 0x68 and
    /// `FUN_140a7a190` with none: the caller releases the foundation) and the point is −1. The
    /// answer is true, so the stop keeps the list; returns whether it counted and the foundation.
    ///
    /// # Errors
    ///
    /// The unit's stop that the motion step does not model.
    pub(crate) fn halt(
        &mut self,
        mover: Option<&mut crate::movement::Mover>,
    ) -> Result<(bool, i32), GenieError> {
        let counted = std::mem::replace(&mut self.added, false);
        if self.move_to.take().is_some()
            && let Some(m) = mover
        {
            m.stop()?;
        }
        self.goal = None;
        self.walk_target = -1;
        self.point = [-1.0; 3];
        self.state = HALTED;
        let target = std::mem::replace(&mut self.target, -1);
        Ok((counted, target))
    }

    /// The search for more work found nothing (`0x140ab0f00`): slot 0x68 with none lets the
    /// foundation go (the caller's) and `set_state(3)` waits (the caller's notify 0x202 and
    /// standing sprite: its unit AI has no orders, or the search would not have run).
    ///
    /// # Errors
    ///
    /// A Build with its `MoveTo` (`set_state`'s teardown is not modelled for it).
    pub const fn no_more_work(&mut self) -> Result<(), GenieError> {
        if self.move_to.is_some() {
            return Err(GenieError::unsupported("a Build waiting with its MoveTo"));
        }
        self.target = -1;
        self.state = IDLE;
        Ok(())
    }

    /// `set_state(1)` and `set_state(13)` (`0x140aae200` cases 1 and 0xd, one case): the prelude
    /// ([`BuildAction::leave`]; out of the builders of `builders`), then the builder takes its
    /// standing sprite (slot 0x68 with its master's `+0x470`, [`BuildStep::stands`], the
    /// caller's) and, with a target, its owner's slot 0x380 hears of it (`FUN_141618d30`: only a
    /// computer player's AI listens).
    fn end(
        &mut self,
        walk: &mut Walk<'_>,
        builders: Option<&mut u8>,
        state: u8,
        step: &mut BuildStep,
    ) -> Result<(), GenieError> {
        self.leave(walk, builders)?;
        self.state = state;
        step.stands = true;
        Ok(())
    }

    /// `set_state(7)` (`0x140aae200` case 7) on a foundation no builder started: clear the
    /// footprint of what may be built over (`FUN_1415c3620` with `param_6`,
    /// [`footprint_clearing`]: the records of what goes go now, the deletes are the caller's
    /// after the update, [`BuildStep::cleared`]), find who stands on it (those just cleared are in
    /// state 7, which the test passes over), and send this builder off it if it stands there
    /// itself.
    fn place(
        &mut self,
        walk: &mut Walk<'_>,
        site: Option<Site<'_>>,
        (waits, look): (&mut u8, Looks<'_>),
        step: &mut BuildStep,
    ) -> Result<(), GenieError> {
        let mut site = site.ok_or(GenieError::unsupported("a foundation of another player"))?;
        // every set_state first takes the builder out of FUN_140aaddb0's building's builders
        if self.added {
            let b = site.builders();
            *b = b.wrapping_sub(1);
            self.added = false;
        }
        self.move_to = None;
        self.goal = None;
        self.walk_target = -1;
        self.state = PLACE;
        if !site.construction.desolid {
            return self.join(walk, &mut site);
        }
        let placed = *walk
            .world
            .get_known(self.target)?
            .ok_or(GenieError::unsupported("a foundation that is gone"))?;
        let clearing = ClearingSite {
            player: placed.player,
            master: placed.master,
            class: placed.class,
            radius: placed.radius,
            at: [placed.position[0], placed.position[1]],
            trees: true,
        };
        let cleared = footprint_clearing(walk.world, &clearing, look.0)?;
        for cl in cleared.iter().filter(|cl| cl.record_goes) {
            if let Some(p) = walk.plan.as_mut() {
                p.pathing.removed(cl.id);
            }
            walk.world.drop_record(cl.id);
        }
        if let Some(p) = walk.plan.as_mut() {
            p.pathing.tiles_changed(walk.world.take_changes());
        }
        let site_ref = &mut site;
        let site = &placed;
        let world = &*walk.world;
        let obstructs = !matches!(site.obstruction_type, 0 | 12);
        let occupants: Vec<i32> = world
            .footprint_occupants(self.target, site, obstructs)?
            .into_iter()
            .filter(|&id| !cleared.iter().any(|cl| cl.id == id))
            .collect();
        step.cleared = cleared;
        step.occupants = occupants
            .iter()
            .copied()
            .filter(|&id| id != walk.me.id)
            .collect();
        if world.foreign_marks(site) {
            return Err(GenieError::unsupported(
                "another player's marks on the footprint",
            ));
        }
        if occupants.is_empty() {
            return self.join(walk, site_ref);
        }
        let [bx, by, _] = walk.me.position;
        let [sx, sy, _] = site.position;
        let inside = (bx - sx).abs() <= site.radius[0] && (by - sy).abs() <= site.radius[1];
        if inside {
            return self.clear(walk);
        }
        // LAB_140aaf5d4: the builder waits a second (set_state 222) up to 21 times
        if MAX_WAITS <= *waits {
            return Err(GenieError::unsupported(
                "a builder gives up on its occupied footprint (event 0x88, set_state 2)",
            ));
        }
        *waits += 1;
        self.state = WAIT;
        self.timer = 1.0;
        Ok(())
    }

    /// `LAB_140aaf9f4`: the builder joins the work: unless walls of other players stand on both
    /// tiles beside it on the footprint's diagonal (`set_state` 2, not modelled), it counts among
    /// the foundation's builders. Its work sprite and sound are not held by the slice.
    fn join(&mut self, walk: &Walk<'_>, site: &mut Site<'_>) -> Result<(), GenieError> {
        let placed = walk
            .world
            .get(self.target)
            .ok_or(GenieError::unsupported("a foundation that is gone"))?;
        let [bx, by] = [walk.me.position[0] as i32, walk.me.position[1] as i32];
        let [tx, mut ty] = [placed.position[0] as i32, placed.position[1] as i32];
        let [rx, ry] = placed.radius;
        if (rx as i32) as f32 == rx && (ry as i32) as f32 == ry {
            if by < ty {
                if tx < bx {
                    ty -= 1;
                }
            } else if ty < by && bx < tx {
                ty -= 1;
            }
        }
        let (dx, dy) = (bx - tx, by - ty);
        if dx.abs() == dy.abs() {
            let (a, b) = beside_diagonal(bx, by, dx, dy);
            let player = placed.player;
            let walled = |[x, y]: [i32; 2]| {
                walk.world
                    .object_on_tile(x, y, |o| o.class == WALL && o.player != player)
            };
            if walled(a) && walled(b) {
                return Err(GenieError::unsupported(
                    "walls beside a diagonal builder (set_state 2)",
                ));
            }
        }
        // FUN_140aaddb0's building counts it: an annex's main building
        if !self.added {
            let b = site.builders();
            *b = b.wrapping_add(1);
            self.added = true;
        }
        Ok(())
    }

    /// The work (state 7, `0x140ab0370` case 6): the builder adds its rate times dt to the
    /// foundation's construction: its work rate times the task's work value, times the owner's
    /// construction rate (attribute 195) when that is above 0 (the Wonder's at most 1.2), each
    /// builder after the first adding a third ([`Construction::work`]). A finished building, a
    /// farm's reseeding and the instant-build cheat (`game+0xc14`) are not modelled.
    fn work(
        &mut self,
        mut walk: Walk<'_>,
        site: Option<Site<'_>>,
        load: Load,
    ) -> Result<(Option<Built>, Dropped, Option<f32>), GenieError> {
        let mut site = site.ok_or(GenieError::unsupported("a foundation of another player"))?;
        let plan = walk
            .plan
            .as_mut()
            .ok_or(GenieError::unsupported("work without the world's planners"))?;
        let mut rate = self.task.work_rate * self.task.work_value * walk.dt;
        let mut factor = plan.construction_rate;
        if site.life.master_id == WONDER && WONDER_RATE < factor {
            factor = WONDER_RATE;
        }
        if 0.0 < factor {
            rate *= factor;
        }
        let n = f32::from(*site.builders());
        if 1.0 < n {
            rate *= ((n - 1.0) * EXTRA_BUILDER + 1.0) / n;
        }
        let (pathing, now) = (&mut *plan.pathing, plan.now);
        let worked = Construction::work(&mut site, rate, walk.world, pathing, now)?;
        let (completed, passed) = match worked {
            Worked::Going => return Ok((None, None, None)),
            Worked::Main {
                amount,
                going: true,
            } => return Ok((None, None, Some(amount))),
            Worked::Main { amount, .. } => (None, Some(amount)),
            Worked::Done => (None, None),
            Worked::Completed(built) => (Some(built), None),
        };
        // a finished foundation with hit points ends the work (its `+0x278`, decoded); one
        // without (deleted, it dies in its own update) leaves the Build as it is
        if 0.0 < site.life.hp {
            let dropped = self.finish(&mut walk, Some(site.builders()), load)?;
            Ok((completed, dropped, passed))
        } else {
            Ok((completed, None, passed))
        }
    }

    /// `set_state(4)` after the walk off the footprint (`0x140aae200` case 4): the goal is the
    /// foundation again and a new `MoveTo` to it (`FUN_140aa0df0`) starts arrived if the footprints
    /// touch, as [`BuildAction::start`]'s. Only a villager returns at once (`0x140ab0370` case
    /// 0xde; any other builder in range would work, state 6).
    fn back(&mut self, walk: &Walk<'_>) -> Result<(), GenieError> {
        let mover = walk
            .mover
            .as_deref()
            .ok_or(GenieError::unsupported("a builder without movement state"))?;
        if walk.world.get(walk.me.id).map(|p| p.class) != Some(VILLAGER) {
            return Err(GenieError::unsupported(
                "a builder other than a villager returns",
            ));
        }
        let site = walk
            .world
            .get_known(self.target)?
            .ok_or(GenieError::unsupported("a foundation that is gone"))?;
        let ground = [mover.stats.radius[0], mover.stats.radius[1]];
        let gap = footprint_gap(walk.me.position, ground, site.position, site.radius);
        *self = Self::start(self.target, site.position, gap, self.task);
        Ok(())
    }

    /// `set_state(223)`: a `MoveTo` to the free tile beside the footprint (`FUN_1411566b0`); it starts
    /// in state 3, as a `MoveTo` to a point is far from it (`FUN_140aa2100`).
    fn clear(&mut self, walk: &Walk<'_>) -> Result<(), GenieError> {
        let mover = walk
            .mover
            .as_deref()
            .ok_or(GenieError::unsupported("a builder without movement state"))?;
        let site = walk.world.get(self.target).expect("place found it");
        let spot = walk
            .world
            .spot_beside(
                walk.me.position,
                (walk.me.id, mover.stats.radius[0]),
                mover.stats.restriction_mask,
                site,
            )?
            .ok_or(GenieError::unsupported(
                "no free tile beside the footprint (set_state 2)",
            ))?;
        if mover.stats.speed == 0.0 {
            return Err(GenieError::unsupported(
                "a builder that cannot move (MoveTo state 13)",
            ));
        }
        self.state = CLEAR;
        self.move_to = Some(MoveTo::Start);
        self.walk_target = -1;
        self.goal = Some([spot[0], spot[1], walk.me.position[2]]);
        Ok(())
    }

    /// Its `MoveTo` returned 4 or 5 in state 4 or 223 (`0x140ab0370` cases 3 and 0xde): slot
    /// 0x68 with none (`FUN_140ab0300`: a counted builder leaves the builders of
    /// `FUN_140aaddb0`'s building, the foundation is let go, [`BuildStep::released`]) and
    /// `set_state(11)`: the prelude ([`BuildAction::leave`]: the `MoveTo`'s teardown stops the
    /// unit), then a quiet `MoveTo` to the action's point (`FUN_140aa0d30`: range 0, the task's
    /// moving sprite; `FUN_140a7a170` with 1), which starts in state 3 (`FUN_140aa2100`: a
    /// `MoveTo` without a target is far from it), in 13 for a unit without speed.
    fn lose_foundation(
        &mut self,
        walk: &mut Walk<'_>,
        site: Option<Site<'_>>,
        step: &mut BuildStep,
    ) -> Result<(), GenieError> {
        let builders = site.map(Site::into_builders);
        self.leave(walk, builders)?;
        if self.target != -1 {
            self.target = -1;
            step.released = true;
        }
        self.state = WALK_POINT;
        self.goal = Some(self.point);
        let moves = walk.mover.as_deref().is_some_and(|m| m.stats.speed != 0.0);
        self.move_to = Some(if moves { MoveTo::Start } else { MoveTo::Failed });
        Ok(())
    }
}

/// What the footprint clearing (`FUN_1415c3620`) reads of the new building: its owner
/// (`param_2`), its master's id (`param_3`), class and radii, its place (`param_4`,
/// `param_5`), and whether the caller is the builder's or a new object's (`param_6`: the trees
/// at most of 1 hit point go too, and each deleted object's record with them).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClearingSite {
    /// The building's owner.
    pub player: usize,
    /// Its master's id.
    pub master: i16,
    /// Its master's class (`+0x410`).
    pub class: i16,
    /// Its master's radii (`+0x5d8`, `+0x5dc`).
    pub radius: [f32; 2],
    /// Its place.
    pub at: [f32; 2],
    /// `param_6`: the builder's call (the Build's `set_state(7)`) or a new object's
    /// (`FUN_14162ce00` with its flag).
    pub trees: bool,
}

/// What the footprint clearing reads of an object on a tile besides its place: its hit points
/// (`+0x278`), its master's height (`+0x5e0`) and type, its state, whether it is a desolid
/// foundation (`+0xc60`) and whether its record went already (`mROOAlreadyCalled`, `+0x686`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClearLook {
    /// `+0x278`.
    pub hp: f32,
    /// Its master's `radius_z` (`+0x5e0`).
    pub height: f32,
    /// `+0x686`.
    pub roo_called: bool,
    /// Its master's type.
    pub kind: u8,
    /// `+0x640`.
    pub state: u8,
    /// `+0xc60`.
    pub desolid: bool,
}

/// An object the footprint clearing deletes, in the tiles' order: a wall (class 27) first goes
/// through the building's delete core (`FUN_1411ccbb0`), then every one is destroyed (slot
/// 0xd0); with `param_6` its record goes after (`FUN_141189670`, then `+0x686` set,
/// `FUN_141189b60`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cleared {
    /// The object.
    pub id: i32,
    /// It is a wall (class 27): `FUN_1411ccbb0` before its destroy.
    pub wall: bool,
    /// Its record goes in the clearing: its destroy's death part drops it (a unit or a tall
    /// building not desolid of type 70 and up below state 3 whose record is still there,
    /// [`crate::turn::combat_dies`], which sets `+0x686`), or with `param_6` `FUN_141189670`
    /// after the destroy does (`+0x686` still clear, and its master tall, COMISS: a NaN height
    /// too, or the object holding its record).
    pub record_goes: bool,
}

/// The flag masters (`FUN_140fde270`: 274, 1689 and 1785), which the footprint clearing leaves
/// alone unless their master has height.
const FLAG_MASTERS: [i16; 3] = [0x112, 0x699, 0x6f9];
/// The tiles the footprint clearing scans each way of the building's place (`DAT_142eb0f48`).
pub(crate) const CLEARING_REACH: f32 = crate::image::build::CLEARING_REACH;
/// The hit points a tree at most has that the builder's clearing takes (`DAT_14330eb8c`).
const STUMP_HP: f32 = crate::image::build::STUMP_HP;
/// The masters whose classes (39, the gate, and 52) take their owner's walls (27) off their
/// footprint, and the tree class (15).
const GATE: i16 = 39;
const CLASS_52: i16 = 52;
const TREE: i16 = 15;

/// `FUN_1415c3620` with `param_7` 0, its choice: the objects whose centre is on the tiles from 4
/// before the building's place to 4 past it (CVTTSS2SI to 16 bits, held on the map), rows then
/// columns, each tile's list in its order (`objects_on`: the ghosts, type 25, are not on it), of
/// another master than the building's, that may be built on (`+0x5e9`), or a wall (27) of the
/// building's owner under a gate (39) or a master of class 52, or with `param_6` a tree (15)
/// of at most 1 hit point (COMISS: a NaN is not), whose box meets the building's (on each axis
/// the gap below the two radii summed; the gap's sign flips only below 0). Of those, a flag
/// master (`FUN_140fde270`) whose height is at most 0 stays; the others go ([`Cleared`]). A
/// building without radii clears nothing (COMISS: a NaN radius is not 0 or less). The game's
/// other branch, `param_7` or the editor's screen mode (`FUN_140c87f20` 4: the objects only leave
/// their tiles, `FUN_140f5f3c0`), never runs in a game: `param_7` is the map generator's
/// (`FUN_140fad9c0`).
///
/// # Errors
///
/// An object on a tile the caller does not hold (`look` gives none), and for the builder's call a
/// frozen object that may be on the tiles ([`Spatial::check_known`]).
pub(crate) fn footprint_clearing(
    world: &Spatial,
    site: &ClearingSite,
    look: &dyn Fn(i32) -> Option<ClearLook>,
) -> Result<Vec<Cleared>, GenieError> {
    use crate::checkpoint::cvttss2si;
    use crate::fmath::below_or_unordered;
    let [rx, ry] = site.radius;
    if !below_or_unordered(0.0, rx) && !below_or_unordered(0.0, ry) {
        return Ok(Vec::new());
    }
    let [w, h] = world.map;
    let range = |v: f32, size: i32| {
        let low = cvttss2si(v - CLEARING_REACH) as i16;
        let high = cvttss2si(v + CLEARING_REACH) as i16;
        let high = if size <= i32::from(high) {
            (size - 1) as i16
        } else {
            high
        };
        (low.max(0), high)
    };
    let (x0, x1) = range(site.at[0], w);
    let (y0, y1) = range(site.at[1], h);
    if x1 < x0 || y1 < y0 {
        return Ok(Vec::new());
    }
    // a stopped player's units may have felled a tree there since they froze: the builder's
    // choice, which reads the trees' hit points, wants the tiles known; the constructor's reads
    // only masters and owners, which they do not change
    if site.trees {
        world.check_known([
            f32::from(x0),
            f32::from(y0),
            f32::from(x1) + 1.0,
            f32::from(y1) + 1.0,
        ])?;
    }
    let under_gate = matches!(site.class, GATE | CLASS_52);
    let mut cleared = Vec::new();
    for y in y0..=y1 {
        for x in x0..=x1 {
            for (id, o) in world.objects_on(i32::from(x), i32::from(y)) {
                if o.master == site.master {
                    continue;
                }
                let own_wall = o.player == site.player && under_gate && o.class == WALL;
                let tree = site.trees && o.class == TREE;
                if o.buildable_on == 0 && !own_wall && !tree {
                    continue;
                }
                let seen = look(id).ok_or(GenieError::unsupported(
                    "a footprint clearing of an object the slice does not hold",
                ))?;
                // the tree's test: COMISS 1.0 against its hit points, JC passes it over (a NaN
                // too)
                if o.buildable_on == 0 && !own_wall && below_or_unordered(STUMP_HP, seen.hp) {
                    continue;
                }
                let dx = (o.position[0] - site.at[0]).abs();
                let dy = (o.position[1] - site.at[1]).abs();
                if !(dx < o.radius[0] + rx && dy < o.radius[1] + ry) {
                    continue;
                }
                // COMISS 0 against the height, JNC: a flag master of height 0 or less stays
                if FLAG_MASTERS.contains(&o.master) && !below_or_unordered(0.0, seen.height) {
                    continue;
                }
                // the destroy's death part (FUN_14121cc40 below state 3) drops the record and sets
                // +0x686 first; then FUN_141189670: COMISS 0 against the height, JC drops the
                // record (a NaN too), else the record held (`+0x658`) does
                let dies = 70 <= seen.kind && seen.state < 3 && !seen.roo_called;
                let destroy_drops =
                    dies && (seen.kind < 80 || (!seen.desolid && 0.0 < seen.height));
                let roo_drops = site.trees
                    && !seen.roo_called
                    && !dies
                    && (below_or_unordered(0.0, seen.height) || o.obstruction.is_some());
                cleared.push(Cleared {
                    id,
                    wall: o.class == WALL,
                    record_goes: destroy_drops || roo_drops,
                });
            }
        }
    }
    Ok(cleared)
}

impl crate::turn::Turns {
    /// What a built building's storage gave its owner leaves (`FUN_1411e63f0`, then
    /// `FUN_1411e6580`, [`crate::turn::storage_leaves`]) outside its update.
    ///
    /// # Errors
    ///
    /// An object the slice does not hold, its own store, and an attribute the owner lacks.
    pub(crate) fn storage_leaves(
        &mut self,
        owner: usize,
        id: i32,
        master: &Master,
    ) -> Result<(), GenieError> {
        let (_, u) = self.object_mut(id).ok_or(NOT_HELD)?;
        for (kind, amount) in crate::turn::storage_leaves(u, master) {
            self.players[owner].add_attribute(kind, amount)?;
        }
        Ok(())
    }

    /// The completion's first part (`FUN_1411c6bd0`, [`Completion::first_part`]) of the
    /// foundation `id` of `owner` outside its update: its owner gains its attributes, then a mule
    /// cart's search runs from state 0 ([`crate::turn::Turns::cart_search`]).
    ///
    /// # Errors
    ///
    /// What the part does not model.
    pub(crate) fn completion_part(
        &mut self,
        owner: usize,
        id: i32,
        master: &Master,
    ) -> Result<(), GenieError> {
        let (_, u) = self.object(id).ok_or(NOT_HELD)?;
        let construction = u.construction.ok_or(NO_CONSTRUCTION)?;
        let completion = Completion {
            master: Some(master),
            annexed: false,
            attribute_204: false,
            mode_680: self.mode_680,
            farm_bonus: crate::turn::farm_bonus(&self.players[owner].attributes),
            masters: self.masters.get(owner),
            computer: self.players[owner].computer,
            ai: u.ai.is_some(),
            main: u.annex_of.is_some(),
            now: self.world_time,
            rice: u.rice,
        };
        let part = completion.first_part(&construction)?;
        // its tracker part (FUN_14104eac0): the foundation leaves the pending lists
        self.track_complete(owner, id, master.id);
        for (kind, amount) in part.attributes {
            self.players[owner].add_attribute(kind, amount)?;
        }
        if part.cart {
            self.cart_search(owner, id, 0)?;
        }
        Ok(())
    }

    /// The construction's start (`FUN_1411e5670`, [`Construction::begin`]) of the foundation
    /// `id` of `owner` outside its update: a desolid one starts, its missiles per volley are its
    /// master's, and its owner watches it ([`crate::turn::Turns::watch_started`]).
    ///
    /// # Errors
    ///
    /// What the start does not model.
    pub(crate) fn construction_starts(&mut self, owner: usize, id: i32) -> Result<(), GenieError> {
        let place = self.place(id).ok_or(NOT_HELD)?;
        let (Place::Live(p, _) | Place::Sleeping(p, _)) = place;
        let master = held(&mut self.players, place).life.master_id;
        let volley = self
            .masters
            .get(p)
            .and_then(|m| m.get(master))
            .map_or(0.0, |m| m.volley_fire);
        let now = self.world_time;
        let u = held(&mut self.players, place);
        let Some(c) = u.construction.as_mut() else {
            return Ok(());
        };
        if Construction::begin(
            id,
            c,
            u.mover.as_mut(),
            &mut self.spatial,
            &mut self.pathing,
            now,
        )? {
            c.started = false;
            u.firepower = volley;
            self.watch_started(owner, id)?;
        }
        Ok(())
    }
}

/// The seconds of work a new object's building gets at once from its owner's slot 0x1c0 with its
/// flag (`FUN_14162ce00`'s `FUN_1411cc430` with `DAT_142eea788`).
const STACK_WORK: f32 = crate::image::build::STACK_WORK;

/// The guard of an object the routines here reach through the object itself (their callers
/// hold it): the slice's lists hold every object the world does.
pub(crate) const NOT_HELD: GenieError =
    GenieError::unsupported("an object the slice does not hold");

/// The guard of a building without its construction: the slice gives every building one.
pub(crate) const NO_CONSTRUCTION: GenieError =
    GenieError::unsupported("a building without its construction");

/// The object at `place`, in its player's live or sleeping list (the world's table,
/// `FUN_1415417a0`, holds both).
pub(crate) fn held(players: &mut [crate::turn::Player], place: Place) -> &mut crate::turn::Unit {
    match place {
        Place::Live(p, i) => &mut players[p].objects[i],
        Place::Sleeping(p, i) => &mut players[p].sleeping[i],
    }
}

impl crate::turn::Turns {
    /// `FUN_1411cc430` outside a builder's update, `amount` seconds on the building `id`: first
    /// each annex (`+0xbc0`..`+0xbd8`) of a building type that is desolid starts
    /// (`FUN_1411cd0d0`, [`crate::turn::Turns::clear_foundations_under`], then `FUN_1411e5670`,
    /// [`crate::turn::Turns::construction_starts`]), then while the building is an annex of a
    /// main building in state 0 with at least 1 hit point (COMISS: a NaN too) the work goes to
    /// the main building, whose annexes start alike. A building in state 0 with at least 1 hit
    /// point (a NaN not) starts if desolid and takes the work ([`Construction::add_work`]); one
    /// that reaches its build time completes (`set_state(2)`, [`crate::turn::Turns::set_state`]).
    ///
    /// # Errors
    ///
    /// What the starts and the completion do not model.
    pub(crate) fn work_building(&mut self, id: i32, amount: f32) -> Result<(), GenieError> {
        let mut at = id;
        loop {
            for annex in self.spatial.annexes_of(at) {
                let Some((q, a)) = self.object(annex) else {
                    continue;
                };
                let desolid = a.construction.is_some_and(|c| c.desolid);
                if self.master_of(q, a.life.master_id)?.kind < 80 || !desolid {
                    continue;
                }
                self.clear_foundations_under(annex)?;
                self.construction_starts(q, annex)?;
            }
            let Some(main) = self.object(at).and_then(|(_, u)| u.annex_of) else {
                break;
            };
            match self.object(main) {
                Some((_, m))
                    if m.life.state == 0 && crate::fmath::at_most_or_unordered(1.0, m.life.hp) =>
                {
                    at = main;
                }
                _ => break,
            }
        }
        let (q, u) = self.object(at).ok_or(NOT_HELD)?;
        if u.life.state != 0 || crate::fmath::below_or_unordered(u.life.hp, 1.0) {
            return Ok(());
        }
        if u.construction.is_some_and(|c| c.desolid) {
            self.clear_foundations_under(at)?;
            self.construction_starts(q, at)?;
        }
        let (q, u) = self.object_mut(at).ok_or(NOT_HELD)?;
        let c = u.construction.as_mut().ok_or(NO_CONSTRUCTION)?;
        if c.add_work(&mut u.life.hp, amount) {
            self.set_state(q, at, 2)?;
        }
        Ok(())
    }

    /// The building `id`'s `set_state(2)` from state 0 outside a builder's update, after its
    /// annexes' (`FUN_1411dbb70`; [`crate::turn::Turns::set_state`] runs them first): the
    /// completion ([`Construction::complete`]), then what it leaves its owner
    /// ([`crate::turn::Turns::built`]).
    ///
    /// # Errors
    ///
    /// What the completion does not model.
    pub(crate) fn complete_building(&mut self, id: i32) -> Result<(), GenieError> {
        let attribute_204 = self
            .players
            .iter()
            .any(|q| q.attributes.get(204).is_some_and(|&v| v != 0.0));
        let place = self.place(id).ok_or(NOT_HELD)?;
        let (Place::Live(p, _) | Place::Sleeping(p, _)) = place;
        let farm_bonus = crate::turn::farm_bonus(&self.players[p].attributes);
        let computer = self.players[p].computer;
        let annexed = !self.spatial.annexes_of(id).is_empty();
        let (mode_680, now) = (self.mode_680, self.world_time);
        let masters = self.masters.get(p);
        let u = held(&mut self.players, place);
        let (ai, main, rice) = (u.ai.is_some(), u.annex_of.is_some(), u.rice);
        let master = masters.and_then(|m| m.get(u.life.master_id));
        let construction = u.construction.as_mut().ok_or(NO_CONSTRUCTION)?;
        let mut site = Site {
            id,
            life: &mut u.life,
            construction,
            mover: u.mover.as_mut(),
            firepower: Some(&mut u.firepower),
            roo_called: Some(&mut u.roo_called),
            store: Some(&mut u.store),
            completion: Completion {
                master,
                annexed,
                attribute_204,
                mode_680,
                farm_bonus,
                masters,
                computer,
                ai,
                main,
                now,
                rice,
            },
            chain: WorkChain::default(),
            main_builders: None,
        };
        let built = Construction::complete(&mut site, &mut self.spatial, &mut self.pathing)?;
        // the completion's first part's tracker part (FUN_1411c6bd0 -> FUN_14104eac0)
        let master_id = held(&mut self.players, place).life.master_id;
        self.track_complete(p, id, master_id);
        self.built(p, id, built)
    }

    /// What a completion leaves the owner `p` of the building `id` ([`Built`]), in the game's
    /// order: its history event (the owner's id, `+0x718`, as `data1`; the game records it
    /// between the storage's attributes and the counts, which it does not read), the attributes
    /// it gains, a mule cart's search (`FUN_1411c6bd0`'s tail), the watch of
    /// a construction that started (`FUN_1411e5670`), the stack unit
    /// ([`crate::turn::Turns::stack_unit`]), the technology (`FUN_1413cdcd0`,
    /// [`crate::turn::Turns::research_built`]) and the destroy of a building that disappears
    /// when built (slot 0xd0, [`crate::turn::Turns::destroy`]).
    ///
    /// # Errors
    ///
    /// What the parts do not model.
    pub(crate) fn built(&mut self, p: usize, id: i32, built: Built) -> Result<(), GenieError> {
        if let Some(kind) = built.history {
            let now = self.world_time;
            self.players[p]
                .history
                .add_event(kind, p as f32, 0.0, 0, now);
        }
        for (kind, amount) in built.attributes {
            self.players[p].add_attribute(kind, amount)?;
        }
        if let Some(state) = built.cart {
            self.cart_search(p, id, state)?;
        }
        if built.started {
            if let Some((_, u)) = self.object_mut(id)
                && let Some(c) = u.construction.as_mut()
            {
                c.started = false;
            }
            self.watch_started(p, id)?;
        }
        if let Some(stack) = built.stack {
            self.stack_unit(p, id, stack)?;
        }
        if let Some(t) = built.tech {
            self.research_built(p, t)?;
        }
        if built.vanishes {
            self.destroy(p, id)?;
        }
        Ok(())
    }

    /// The stack unit (`+0x986`) of the completed building `building` of player `p`
    /// (`FUN_1411dbb70`'s block): its owner's slot 0x1c0 makes it at the building's place with
    /// its flag ([`crate::turn::Turns::make_new_object`]); a new building takes the old one's
    /// command queue (the old unit AI's slot 0x218, its orders not empty, then slot 0x1f8: the
    /// new one's slot 0x208 per order, `FUN_140c15d00`) and its gather point (`+0xc30`..: a
    /// target that is the old building names the new one; a target the world no longer holds
    /// keeps the point alone, and with x or y −1 none); then the new object loses the hit points
    /// the old one lacked. Not modelled: the attacking units of the foundation the world still
    /// holds (`+0xce0`, [`crate::turn::Unit::attacking_units`]: slot 0x140 on each with the new
    /// object, a repeat of the last skipped) and a gate's stack gate (slot 0x670 with the old
    /// one's lock). Not held: `+0xcb0` (the new id) and the local
    /// player's tracker (`FUN_14165c2a0`, `FUN_14165c470`).
    ///
    /// # Errors
    ///
    /// What the parts do not model.
    pub(crate) fn stack_unit(
        &mut self,
        p: usize,
        building: i32,
        stack: StackUnit,
    ) -> Result<(), GenieError> {
        let (_, old) = self.object(building).ok_or(NOT_HELD)?;
        let (orders, gather) = (
            old.ai
                .as_ref()
                .map(|ai| ai.building_orders.clone())
                .unwrap_or_default(),
            old.gather_point,
        );
        let old_class = self.master_of(p, old.life.master_id)?.class;
        // its attacking units (+0xce0) the world still holds each hear of the new object (their
        // slot 0x140), which is not modelled
        let enemies = old
            .attacking_units
            .iter()
            .any(|&id| self.object(id).is_some());
        if enemies {
            return Err(GenieError::unsupported(
                "a stack unit's completion with other players' units on it (+0xce0)",
            ));
        }
        let Some(new) = self.make_new_object(p, stack.master, stack.at, true)? else {
            return Ok(());
        };
        let (q, nu) = self.object(new).ok_or(NOT_HELD)?;
        let new_master = self.master_of(q, nu.life.master_id)?;
        let mut point = None;
        let building_made = new_master.kind == BUILDING_TYPE;
        if building_made {
            if old_class == CLASS_39 && new_master.class == CLASS_39 {
                return Err(GenieError::unsupported(
                    "a gate's stack gate (slot 0x670 with the old one's lock)",
                ));
            }
            // a gather point on the old building names the new one; one whose target the world
            // no longer holds keeps its point, none at x or y −1 (compared as floats)
            point = gather.map(|g| {
                let target = if g.target == building { new } else { g.target };
                match self.object(target) {
                    Some((_, t)) => Some(crate::turn::GatherPoint {
                        point: [t.position[0], t.position[1], 0.0],
                        target,
                        target_master: t.life.master_id,
                    }),
                    None => (g.point[0] != -1.0 && g.point[1] != -1.0).then_some(
                        crate::turn::GatherPoint {
                            point: [g.point[0], g.point[1], 0.0],
                            target: -1,
                            target_master: -1,
                        },
                    ),
                }
            });
        }
        let (_, nu) = self.object_mut(new).ok_or(NOT_HELD)?;
        if building_made {
            if let Some(ai) = nu.ai.as_mut().filter(|_| !orders.is_empty()) {
                ai.building_orders.extend(orders);
            }
            if let Some(g) = point {
                nu.gather_point = g;
            }
        }
        nu.life.hp -= stack.hp_lost;
        Ok(())
    }

    /// The player's slot 0x1c0 (`FUN_14162ce00`) for `master` at `at`: the master's slot 0x40
    /// makes the object (none for a master the player has not got, `FUN_141666a60`); with its
    /// flag its footprint is cleared with `param_6` ([`crate::turn::Turns::clear_footprint`]) and
    /// a building is worked 10000 seconds at once ([`crate::turn::Turns::work_building`]: a stack
    /// unit completes there); a master that gathers by itself joins the visible-resource lists
    /// ([`crate::turn::Turns::made_new_object`]). The editor's part (mode 4) never runs in a game.
    /// Its id.
    ///
    /// # Errors
    ///
    /// A master of another type than a building (only stack units come here), and what the parts
    /// do not model.
    pub(crate) fn make_new_object(
        &mut self,
        p: usize,
        master: i16,
        at: [f32; 3],
        flagged: bool,
    ) -> Result<Option<i32>, GenieError> {
        let Some(made) = self.masters.get(p).and_then(|m| m.get(master)).cloned() else {
            return Ok(None);
        };
        if made.kind != BUILDING_TYPE {
            return Err(GenieError::unsupported(
                "the player's slot 0x1c0 for an object other than a building",
            ));
        }
        let ai = Ok(crate::unit_ai::factory_type(
            made.class,
            master,
            made.volley_missile.unwrap_or(-1),
            made.combat_level,
        ));
        let (id, _) = self.make_building(p, master, ai, at)?;
        if flagged {
            self.clear_footprint(p, master, [at[0], at[1]], true)?;
            self.work_building(id, STACK_WORK)?;
        }
        if made.auto_gather {
            if self.players[p].objects.last().map(|u| u.id) != Some(id) {
                return Err(GenieError::unsupported(
                    "an auto-gathered new building with annexes (its visible resources)",
                ));
            }
            self.made_new_object(p)?;
        }
        Ok(Some(id))
    }
}

/// `FUN_140c5dcd0`: the build orders (`0x2be`) of `ai`'s queue whose foundation is gone or no
/// longer in state 0 leave it; `site` is the Build's own foundation with its state now, the
/// others' states are as the update started (`build_targets`, None for one that is gone).
fn drop_done_orders(ai: &mut UnitAi, site: Option<(i32, u8)>, build_targets: &[(i32, Option<u8>)]) {
    ai.orders.retain(|o| {
        if o.order != BUILD_ORDER {
            return true;
        }
        let state = match site {
            Some((id, state)) if id == o.target => Some(state),
            _ => build_targets
                .iter()
                .find(|&&(id, _)| id == o.target)
                .and_then(|&(_, s)| s),
        };
        state == Some(0)
    });
}

/// The two tiles beside a builder on a footprint's diagonal (`LAB_140aafc09`'s arguments).
const fn beside_diagonal(bx: i32, by: i32, dx: i32, dy: i32) -> ([i32; 2], [i32; 2]) {
    if dx < 1 {
        if dx < 0 && dy < 0 {
            ([bx + 1, by], [bx, by + 1])
        } else if dx < 0 && 0 < dy {
            ([bx, by - 1], [bx + 1, by])
        } else {
            ([bx - 1, by], [bx, by + 1])
        }
    } else if dy < 1 {
        ([bx - 1, by], [bx, by + 1])
    } else {
        ([bx, by - 1], [bx - 1, by])
    }
}

impl Construction {
    /// `FUN_1411cc430`: `amount` seconds of work on a foundation. An annex whose main building
    /// takes the work leaves it all to the caller ([`WorkChain::main`], [`Worked::Main`]: the
    /// main building's result, its start's build points first). Otherwise its annexes of a
    /// building type that are desolid start first ([`AnnexStart`]); then a foundation in state 0
    /// with at least 1 hit point (else the work is done) starts with the first work
    /// ([`Construction::begin`]), each adds its share of the master's hit points (all of them
    /// without a build time) up to the build time, and the work that reaches it completes the
    /// building ([`Construction::complete`]).
    pub fn work(
        site: &mut Site<'_>,
        amount: f32,
        world: &mut Spatial,
        pathing: &mut Pathing,
        now: u32,
    ) -> Result<Worked, GenieError> {
        if let Some(main) = site.chain.main {
            let MainWork {
                mut construction,
                mut hp,
            } = main;
            if construction.desolid {
                construction.progress = f32::from_bits(STARTED);
            }
            let going = !construction.add_work(&mut hp, amount);
            return Ok(Worked::Main { amount, going });
        }
        for a in &mut site.chain.annexes {
            if a.construction.desolid {
                world.started(a.id);
                if Self::begin(
                    a.id,
                    a.construction,
                    a.mover.as_deref_mut(),
                    world,
                    pathing,
                    now,
                )? {
                    *a.firepower = a.volley;
                }
            }
        }
        // COMISS: a NaN hit point count is not worked on either
        if site.life.state != 0 || crate::fmath::below_or_unordered(site.life.hp, 1.0) {
            return Ok(Worked::Done);
        }
        let c = &mut *site.construction;
        if c.desolid {
            // FUN_1411cd0d0: the desolid foundations it overlaps go after the builder's update
            // (Turns::clear_foundations_under), then FUN_1411e5670
            world.started(site.id);
            if Self::begin(site.id, c, site.mover.as_deref_mut(), world, pathing, now)?
                && let (Some(f), Some(m)) = (site.firepower.as_deref_mut(), site.completion.master)
            {
                *f = m.volley_fire;
            }
        }
        if !c.add_work(&mut site.life.hp, amount) {
            return Ok(Worked::Going);
        }
        Self::complete(site, world, pathing).map(Worked::Completed)
    }

    /// `FUN_1411cc430`'s work of `amount` seconds on a foundation in state 0 with at least 1 hit
    /// point: at most up to its build time, it adds its share of its master's hit points `hp`
    /// (all of them without a build time of at least 1), never past them; true when its build
    /// points reach its build time.
    pub(crate) fn add_work(&mut self, hp: &mut f32, amount: f32) -> bool {
        let mut amount = amount;
        let build_time = f32::from(self.build_time);
        let hit_points = f32::from(self.hit_points);
        if build_time < self.progress + amount {
            amount = build_time - self.progress;
        }
        let new = if 1.0 <= build_time {
            (amount * hit_points) / build_time + *hp
        } else {
            hit_points
        };
        *hp = if hit_points < new { hit_points } else { new };
        let before = self.progress;
        self.progress = amount + before;
        crate::fmath::at_most_or_unordered(build_time, amount + before)
    }

    /// The construction completes: the building's `set_state(2)` (`FUN_1411dbb70` from state 0;
    /// its annexes' come first, the caller's), then `FUN_14152bcb0`, whose part for players with
    /// attribute 204 is not modelled. The completion's first part (`FUN_1411c6bd0`,
    /// [`Completion::first_part`]) and a desolid one's construction start (`FUN_1411e5670`,
    /// [`Construction::begin`]: [`Built::started`]); then the block for state 2: its build points
    /// hold its build time; its master's storage of modes 4, 8 and 64 goes to its owner (slot
    /// 0xf8, returned); its sight swaps from its footprint to its line of sight (slots 0x1d8,
    /// 0x1d0; not for classes 51 and 54, whose `+0xca1` the constructor set); it takes its
    /// standing sprite (not held); a stack unit (`+0x986`) drops its record and mesh shape
    /// (`FUN_141055b40`), lets its tiles go (`FUN_140f65580`, a building on a tile that is no
    /// annex) and sets `+0x686`; its owner makes the stack unit (the caller's, [`Built::stack`]);
    /// on snow it gets its snow flag; its foundation terrain covers its clearance (terrain 0x3f
    /// under master 1889's pasture, [`crate::terrain::Terrain::cover`]; the tile it stands on,
    /// `+0x2e8`, keeps no terrain to restore in the slice); its owner's attributes 98 and 165
    /// grow by its paid cost (`0x1411dd3ee`: the sum of its cost entries flagged paid) unless it
    /// is an annex or has a stack unit; the castle (82) and the wonder (276) instead record
    /// their building in their owner's history ([`Built::history`]), count in attributes 134
    /// and 173, 42 and 174, and add their paid cost to 184 (an annex too); the monastery (104)
    /// counts in 52; then the combat object's and the base `set_state`
    /// (`FUN_14121cc40`, `FUN_14119d710`) put it in state 2. The local player's sound (slot
    /// 0x4c0), the event 0x69 (`FUN_140c898a0`), the owner's AI (slot 0x388) and the snow decal
    /// are not held. Last, the master's technology is researched (`FUN_1413cdcd0` with `param_3`
    /// 0 and no building; [`Built::tech`], the caller's), and a master that disappears when built
    /// is destroyed (slot 0xd0, the caller's, [`Built::vanishes`]; its hit points are 0 already,
    /// as the builder's test after the work reads them). A master with object flag 0x40 and a
    /// foundation terrain (its tiles' classes again, `FUN_141055790`) is not modelled; a wall's (`adjacent_mode`) facets, its own and its neighbours' (`FUN_1411ce890`), are not
    /// held.
    ///
    /// # Errors
    ///
    /// What it does not model, and a building without its master or place.
    pub fn complete(
        site: &mut Site<'_>,
        world: &mut Spatial,
        pathing: &mut Pathing,
    ) -> Result<Built, GenieError> {
        /// The terrain the pasture master 1889's stack unit on mangrove or rice terrain lays
        /// (`0x3f`).
        const RICE_PASTURE_TERRAIN: u8 = 0x3f;
        let id = site.id;
        let done = site.completion;
        let master = done.master.ok_or(GenieError::unsupported(
            "a completion of a master the slice does not have",
        ))?;
        let building = master.building.ok_or(GenieError::unsupported(
            "a completion of a master that is no building",
        ))?;
        if done.attribute_204 {
            return Err(crate::diplomacy::REVEAL_UNITS);
        }
        // FUN_1411c6bd0 (its build points are its build time already: FUN_1411cc430 wrote them)
        let first = done.first_part(site.construction)?;
        let mut attributes = first.attributes;
        // FUN_1411e5670 while still in state 0 (2 lies within 1 to 6)
        let started = Self::begin(
            id,
            site.construction,
            site.mover.as_deref_mut(),
            world,
            pathing,
            done.now,
        )?;
        if started && let Some(f) = site.firepower.as_deref_mut() {
            *f = master.volley_fire;
        }
        // a wall's (adjacent mode) facets and its neighbours' (FUN_1411ce890) are not held
        if 0 <= building.foundation_terrain && master.object_flags & 0x40 != 0 {
            return Err(GenieError::unsupported(
                "a completion of a master of object flag 0x40 (its tiles' classes, FUN_141055790)",
            ));
        }
        let c = &mut *site.construction;
        c.progress = f32::from(c.build_time);
        for &(kind, amount, flag) in &master.storage {
            if kind < 0 || amount <= 0.0 {
                continue;
            }
            if matches!(flag, 4 | 8 | 64) {
                attributes.push((kind, amount));
            }
        }
        // the entries of mode 16 go to its own store (FUN_1411abad0: an entry from 0)
        for &(kind, amount, flag) in &master.storage {
            if kind < 0 || amount <= 0.0 || flag & 0x10 == 0 {
                continue;
            }
            if let Some(store) = site.store.as_deref_mut() {
                match store.iter_mut().find(|e| e.0 == kind) {
                    Some(e) => e.1 += amount,
                    None => store.push((kind, amount)),
                }
            }
        }
        let placed = *world.get(id).ok_or(GenieError::unsupported(
            "a completed building without a place",
        ))?;
        if !matches!(master.class, 51 | 54)
            && let Some(v) = world.sight.as_mut()
        {
            v.construction_completed(id, placed.position, placed.radius);
        }
        let stack_master = if master.id == MASTER_1889 && 0 <= building.stack_unit && done.rice {
            RICE_PASTURE
        } else {
            building.stack_unit
        };
        let mut stack = None;
        if 0 <= building.stack_unit {
            pathing.removed(id);
            world.drop_record(id);
            pathing.tiles_changed(world.take_changes());
            if !done.main
                && let Some(v) = world.sight.as_mut()
            {
                v.unlink_building(placed.position, master.radius);
            }
            stack = Some(StackUnit {
                master: stack_master,
                at: placed.position,
                hp_lost: f32::from(master.hit_points) - site.life.hp,
            });
            if let Some(r) = site.roo_called.as_deref_mut() {
                *r = true;
            }
        }
        let terrain = world.terrain.as_mut().ok_or(GenieError::unsupported(
            "a completion without the map's terrain",
        ))?;
        let [x, y, _] = placed.position;
        if !c.snow && 0 <= x as i32 && 0 <= y as i32 && terrain.snow_under(x as i32, y as i32)? {
            c.snow = true;
        }
        if 0 <= building.foundation_terrain {
            let laid = if stack_master == RICE_PASTURE {
                RICE_PASTURE_TERRAIN
            } else {
                building.foundation_terrain as u8
            };
            terrain.cover(&crate::terrain::Foundation {
                position: placed.position,
                clearance: master.clearance,
                terrain: laid,
                snow: c.snow,
                master: master.id,
                class: master.class,
                terrain_bonus: master.terrain_bonus,
                mode_680: done.mode_680,
            })?;
        }
        // the castle's and the wonder's history event (0x1411dd094), the counts (slot 0xf8 with
        // 1.0, 0x14330eb8c)
        let history = match master.id {
            CASTLE => Some(crate::history::CASTLE_BUILT),
            WONDER => Some(crate::history::WONDER_BUILT),
            _ => None,
        };
        match master.id {
            MONASTERY => attributes.push((MONASTERIES, 1.0)),
            WONDER => attributes.extend([(WONDERS, 1.0), (WONDERS_BUILT, 1.0)]),
            CASTLE => attributes.extend([(CASTLES, 1.0), (CASTLES_BUILT, 1.0)]),
            _ => {}
        }
        // its paid cost: the cost entries of a resource flagged paid (`+0x880`), summed as shorts
        let paid: i32 = master
            .cost
            .iter()
            .filter(|&&(resource, _, paid)| 0 <= resource && paid)
            .map(|&(_, amount, _)| i32::from(amount))
            .sum();
        if matches!(master.id, CASTLE | WONDER) {
            attributes.push((CASTLE_WONDER_COST, paid as f32));
        } else if !done.main && building.stack_unit == -1 {
            attributes.push((98, paid as f32));
            attributes.push((165, paid as f32));
        }
        let before = site.life.state;
        site.life.state = 2;
        world.sight_state(id, 2);
        let vanishes = building.disappears_when_built;
        if vanishes {
            site.life.hp = 0.0;
        }
        Ok(Built {
            attributes,
            history,
            tech: (0 <= building.tech).then_some(building.tech),
            cart: first.cart.then_some(before),
            stack,
            vanishes,
            started,
        })
    }

    /// The construction starts (`FUN_1411e5670`) on a desolid foundation (one not desolid, or
    /// without its own master copy `+0x6c0`, which a desolid one always holds, returns at once,
    /// false): its master is its owner's again (the same in the slice), its missiles per volley
    /// its master's (`+0xa00`, the caller's: true); it is no longer desolid, holds a sliver of
    /// progress, and gets its obstruction record (`FUN_141056460`: the tiles it marks, which the
    /// planners see, and its shape in the mesh); its footprint comes into its player's sight and
    /// its tiles link to its anchor again (`FUN_140f73900`); its owner watches it
    /// (`FUN_141090430` when `FUN_1411cc980`, the caller's through [`Construction::started`]);
    /// its last move time becomes the world's time plus 1. Not held: its damage sprites (made
    /// for a player not allied with the local one), the tall count of its tile (`FUN_140f64540`)
    /// and `+0xca4`.
    ///
    /// # Errors
    ///
    /// A foundation the world does not hold.
    pub(crate) fn begin(
        id: i32,
        c: &mut Self,
        mover: Option<&mut crate::movement::Mover>,
        world: &mut Spatial,
        pathing: &mut Pathing,
        now: u32,
    ) -> Result<bool, GenieError> {
        if !c.desolid {
            return Ok(false);
        }
        let placed = *world
            .get(id)
            .ok_or(GenieError::unsupported("a foundation that is gone"))?;
        c.desolid = false;
        c.started = true;
        c.progress = f32::from_bits(STARTED);
        if let Some(v) = world.sight.as_mut() {
            v.construction_started(id, placed.position, placed.radius);
        }
        // a master of obstruction kind 0 or 12, or of class 0, gets no record (FUN_141056460)
        if c.record.recorded() {
            world.add_record(id, c.record);
        }
        pathing.tiles_changed(world.take_changes());
        if let Some(p) = world.get(id) {
            pathing.moved(id, p, false);
        }
        if let Some(m) = mover {
            m.last_move = now.wrapping_add(1);
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An object of `master` (class `class`, radius `r`, `buildable` on) of player 1 at `x`, `y`.
    fn on_map(
        x: f32,
        y: f32,
        master: i16,
        class: i16,
        r: f32,
        buildable: u8,
    ) -> crate::spatial::Placed {
        crate::spatial::Placed {
            position: [x, y, 0.0],
            radius: [r, r],
            class,
            player: 1,
            master,
            obstruction_type: 0,
            buildable_on: buildable,
            tall: false,
            speed: 0.0,
            unknown: None,
            obstruction: None,
        }
    }

    /// What `FUN_1415c3620` reads besides the place: hit points `hp` and height `height` of a
    /// live static object.
    const fn seen(hp: f32, height: f32) -> ClearLook {
        ClearLook {
            hp,
            height,
            roo_called: false,
            kind: 10,
            state: 2,
            desolid: false,
        }
    }

    /// `FUN_1415c3620` for a house (master 70, radius 1) of player 1 at (10.5, 10.5): rows, then
    /// columns, then each tile's list: two decorations on one tile go in the list's order; a
    /// wall of the owner only under a gate (class 39); a stump (a tree of at most 1 hit point)
    /// only for the builder's call; a flag master (274) of height 0 stays, one with height goes;
    /// what does not overlap, a live tree and the house's own master stay.
    #[test]
    fn the_footprint_clearing_takes_buildable_objects_in_the_tiles_order() {
        use crate::spatial::Tiles;
        let mut world = Spatial::with_tiles([20, 20], Tiles::new(20, 20, vec![0; 400]));
        let objects = [
            (5, on_map(10.5, 10.5, 1358, 5, 0.5, 1), seen(1.0, 0.0)),
            (3, on_map(10.2, 10.2, 1351, 5, 0.5, 1), seen(1.0, 0.0)),
            (8, on_map(11.2, 10.5, 349, TREE, 0.5, 0), seen(0.5, 1.0)),
            (9, on_map(11.5, 9.5, 350, TREE, 0.5, 0), seen(100.0, 1.0)),
            (
                11,
                on_map(11.7, 9.7, 350, TREE, 0.5, 0),
                seen(f32::NAN, 1.0),
            ),
            (4, on_map(9.5, 10.5, 72, WALL, 0.5, 0), seen(100.0, 1.0)),
            (6, on_map(10.2, 11.5, 0x112, 11, 0.5, 1), seen(1.0, 0.0)),
            (7, on_map(11.5, 11.5, 0x112, 11, 0.5, 1), seen(1.0, 2.0)),
            (10, on_map(13.6, 10.5, 1358, 5, 0.5, 1), seen(1.0, 0.0)),
            (12, on_map(10.6, 10.6, 70, 3, 1.0, 1), seen(1.0, 0.0)),
        ];
        for (id, p, _) in &objects {
            world.insert(*id, *p);
        }
        let look = |id: i32| objects.iter().find(|o| o.0 == id).map(|o| o.2);
        let site = |class: i16, trees: bool| ClearingSite {
            player: 1,
            master: 70,
            class,
            radius: [1.0, 1.0],
            at: [10.5, 10.5],
            trees,
        };
        let ids = |world: &Spatial, s: ClearingSite| -> Vec<i32> {
            footprint_clearing(world, &s, &look)
                .unwrap()
                .iter()
                .map(|c| c.id)
                .collect()
        };
        assert_eq!(ids(&world, site(3, false)), [5, 3, 7]);
        assert_eq!(ids(&world, site(3, true)), [5, 3, 8, 7]);
        assert_eq!(ids(&world, site(GATE, false)), [4, 5, 3, 7]);
        // a wall of another player stays under a gate
        world.insert(
            4,
            crate::spatial::Placed {
                player: 2,
                ..objects[5].1
            },
        );
        assert_eq!(ids(&world, site(GATE, false)), [5, 3, 7]);
        // a master without radii clears nothing; a NaN radius is not 0 or less
        let mut none = site(3, true);
        none.radius = [0.0, 0.0];
        assert_eq!(ids(&world, none), Vec::<i32>::new());
        none.radius = [f32::NAN, 0.0];
        assert_eq!(ids(&world, none), Vec::<i32>::new());
    }

    /// The record each cleared object loses in the clearing ([`Cleared::record_goes`]): with
    /// `param_6`, a tall master's or a recorded object's (`FUN_141189670`, a NaN height too), not
    /// one whose record went (`+0x686`); a unit below state 3 loses it in its destroy's death
    /// part whatever the call; a building only when it is tall and not desolid.
    #[test]
    fn a_cleared_object_loses_its_record_by_its_type_and_the_call() {
        use crate::spatial::Tiles;
        let mut world = Spatial::with_tiles([20, 20], Tiles::new(20, 20, vec![0; 400]));
        let looks = [
            (1, seen(1.0, 1.0)),
            (2, seen(1.0, f32::NAN)),
            (
                3,
                ClearLook {
                    roo_called: true,
                    ..seen(1.0, 1.0)
                },
            ),
            (4, seen(1.0, 0.0)),
            (
                5,
                ClearLook {
                    kind: 70,
                    ..seen(1.0, 0.0)
                },
            ),
            (
                6,
                ClearLook {
                    kind: 80,
                    desolid: true,
                    ..seen(1.0, 1.0)
                },
            ),
            (
                7,
                ClearLook {
                    kind: 80,
                    ..seen(1.0, 1.0)
                },
            ),
            (
                8,
                ClearLook {
                    kind: 70,
                    state: 5,
                    ..seen(1.0, 0.0)
                },
            ),
        ];
        for (k, (id, _)) in looks.iter().enumerate() {
            let x = 9.5 + 0.1 * k as f32;
            world.insert(*id, on_map(x, 10.5, 1358, 5, 0.5, 1));
        }
        let look = |id: i32| looks.iter().find(|o| o.0 == id).map(|o| o.1);
        let goes = |trees: bool| -> Vec<i32> {
            let s = ClearingSite {
                player: 1,
                master: 70,
                class: 3,
                radius: [1.0, 1.0],
                at: [10.5, 10.5],
                trees,
            };
            footprint_clearing(&world, &s, &look)
                .unwrap()
                .iter()
                .filter(|c| c.record_goes)
                .map(|c| c.id)
                .collect()
        };
        assert_eq!(goes(true), [1, 2, 5, 7]);
        assert_eq!(goes(false), [5, 7]);
    }

    /// The completion's first part (`FUN_1411c6bd0`) of a farm whose build points reached its
    /// build time: attribute 237's amount for the resource of 238 only when 239 is 0 or more too
    /// and names a master the owner does not have; a pasture is not modelled; a mule cart's
    /// search follows with a unit AI only.
    #[test]
    fn a_farm_s_bonus_wants_both_attributes_238_and_239() {
        let farm = Master {
            id: 50,
            class: FARM,
            ..Master::default()
        };
        let masters = crate::masters::Masters::new([Master {
            id: 68,
            ..Master::default()
        }]);
        let built = Construction {
            progress: 15.0,
            desolid: false,
            builders: 1,
            hit_points: 480,
            build_time: 15,
            record: Obstruction { kind: 0, class: 0 },
            snow: false,
            sprite_override: false,
            started: false,
        };
        let part = |bonus: [f32; 3], c: &Construction| {
            Completion {
                master: Some(&farm),
                annexed: false,
                attribute_204: false,
                mode_680: 0,
                farm_bonus: bonus,
                masters: Some(&masters),
                computer: false,
                ai: false,
                main: false,
                now: 0,
                rice: false,
            }
            .first_part(c)
        };
        assert_eq!(
            part([45.0, 0.0, -1.0], &built).unwrap().attributes,
            Vec::new()
        );
        assert_eq!(
            part([45.0, -1.0, 2.0], &built).unwrap().attributes,
            Vec::new()
        );
        assert_eq!(
            part([45.0, 0.0, 2.0], &built).unwrap().attributes,
            [(0, 45.0)]
        );
        assert_eq!(
            part([45.0, 0.0, 68.0], &built),
            Err(GenieError::unsupported(
                "a farm's bonus near a master (FUN_1411d3cd0)"
            ))
        );
        // not built yet: nothing
        let going = Construction {
            progress: 14.5,
            ..built
        };
        assert_eq!(
            part([45.0, 0.0, 2.0], &going).unwrap().attributes,
            Vec::new()
        );
        // a cart (a task of action type 0x99) with a unit AI searches
        let cart = Master {
            tasks: vec![crate::masters::Task {
                action_type: CART_TASK,
                ..crate::masters::Task::default()
            }],
            ..Master::default()
        };
        let searches = |ai: bool| {
            Completion {
                master: Some(&cart),
                annexed: false,
                attribute_204: false,
                mode_680: 0,
                farm_bonus: [0.0; 3],
                masters: None,
                computer: false,
                ai,
                main: false,
                now: 0,
                rice: false,
            }
            .first_part(&built)
            .unwrap()
            .cart
        };
        assert_eq!((searches(true), searches(false)), (true, false));
    }

    /// A villager's Build task: range 0.1, work value 1, work rate 1.
    const TASK: BuildTask = BuildTask {
        range: 0.1,
        work_value: 1.0,
        work_rate: 1.0,
    };

    /// MarvinBlue/426048731's house 10028 as BUILD placed it: a foundation of master 70.
    fn house() -> crate::spatial::Placed {
        crate::spatial::Placed {
            position: [30.0, 88.0, 0.0],
            radius: [1.0, 1.0],
            class: 3,
            player: 2,
            master: 70,
            obstruction_type: 2,
            buildable_on: 0,
            tall: true,
            speed: 0.0,
            unknown: None,
            obstruction: None,
        }
    }

    /// `FUN_140aaddb0`: a Build on a building annex (`+0xbb8`) counts its builder in the main
    /// building. The join (`0x140aae200` case 7) raises the main building's count and leaves the
    /// annex's; the work's extra-builder factor (`0x140ab0370` case 6) reads the main building's
    /// two builders; the prelude of the `set_state(4)` of slot 0x48 on the annex again
    /// (`FUN_140ab1b30`, [`BuildAction::retake`]) lowers the main building's count.
    #[test]
    fn a_builder_on_an_annex_counts_among_its_main_building_s_builders() {
        fn update(
            build: &mut BuildAction,
            (life, annex, main): (&mut Lifecycle, &mut Construction, &mut u8),
            world: &mut Spatial,
        ) {
            let (mut pathing, mut object) = (Pathing::default(), crate::rng::Well512::default());
            let groups = crate::group::UnitGroups::default();
            let site = Site {
                id: 10029,
                life,
                construction: annex,
                mover: None,
                firepower: None,
                roo_called: None,
                store: None,
                completion: Completion {
                    master: None,
                    annexed: false,
                    attribute_204: false,
                    mode_680: 0,
                    farm_bonus: [0.0, -1.0, -1.0],
                    masters: None,
                    computer: false,
                    ai: false,
                    main: true,
                    now: 0,
                    rice: false,
                },
                chain: WorkChain::default(),
                main_builders: Some(main),
            };
            let walk = Walk {
                mover: None,
                me: Me {
                    id: 5,
                    position: [32.0, 88.0, 0.0],
                    group: -1,
                    action_state: build.state,
                },
                dt: 0.03,
                world,
                plan: Some(crate::movement::Planning {
                    pathing: &mut pathing,
                    now: 0,
                    computer: false,
                    object: &mut object,
                    construction_rate: 0.0,
                    groups: &groups,
                }),
            };
            let builder = Builder {
                class: VILLAGER,
                master: 83,
                held: 0.0,
                held_kind: -1,
                drop_site: false,
                ai: None,
                waits: &mut 0,
                build_targets: &[],
                look: Looks(&|_| None),
                walk_target_state: Some(0),
            };
            let _ = build.update(walk, Some(site), builder).unwrap();
        }
        let mut world = Spatial::default();
        world.insert(
            10029,
            crate::spatial::Placed {
                player: 1,
                ..house()
            },
        );
        let mut life = Lifecycle {
            state: 0,
            hp: 1.0,
            master_id: 1192,
            ..Lifecycle::default()
        };
        let mut annex = Construction {
            progress: 1.0,
            desolid: false,
            builders: 0,
            hit_points: 550,
            build_time: 25,
            record: Obstruction { kind: 2, class: 2 },
            snow: false,
            sprite_override: false,
            started: false,
        };
        // another builder works on the main building
        let mut main = 1u8;
        let mut build = BuildAction {
            state: WAIT,
            move_to: None,
            timer: 0.01,
            ..BuildAction::start(10029, [0.0; 3], 0.0, TASK)
        };
        update(&mut build, (&mut life, &mut annex, &mut main), &mut world);
        assert_eq!(
            (build.state, build.added, main, annex.builders),
            (PLACE, true, 2, 0)
        );
        update(&mut build, (&mut life, &mut annex, &mut main), &mut world);
        let rate = 1.0f32 * 1.0 * 0.03;
        let n = f32::from(main);
        assert_eq!(
            annex.progress,
            1.0 + rate * (((n - 1.0) * EXTRA_BUILDER + 1.0) / n)
        );
        build
            .retake(([0.0; 3], 0.0), None, Some(&mut main), false)
            .unwrap();
        assert_eq!((build.state, build.added, main), (MOVE, false, 1));
    }

    /// records/425906646's update 3145: a builder carrying wood when its lumber camp completes
    /// drops it there (`FUN_140aadc50`); food kinds become food; at a house it keeps its load.
    #[test]
    fn a_builder_drops_its_load_at_a_drop_site_it_completes() {
        let mut world = Spatial::default();
        let mut finish = |kind: i16, drop_site: bool| {
            let mut b = BuildAction {
                state: PLACE,
                move_to: None,
                ..BuildAction::start(31500, [0.0; 3], 0.0, TASK)
            };
            let mut walk = Walk {
                mover: None,
                me: Me {
                    id: 1,
                    position: [0.0; 3],
                    group: -1,
                    action_state: 0,
                },
                dt: 0.02,
                world: &mut world,
                plan: None,
            };
            let load = Load {
                amount: 6.881_128,
                kind,
                drop_site,
            };
            let dropped = b.finish(&mut walk, None, load).unwrap();
            assert_eq!(b.state, FINISHED);
            dropped
        };
        assert_eq!(finish(1, true), Some((1, 6.881_128)));
        assert_eq!(finish(16, true), Some((0, 6.881_128)));
        assert_eq!(finish(1, false), None);
        assert_eq!(finish(-1, true), None);
    }

    /// records/425906646's update 4804: builders of players 4 and 5 told to build another
    /// foundation: even one at work (state 6) starts over on the new one (state 4), no longer
    /// among builders, its `MoveTo` arrived when the footprints touch.
    /// Slot 0x38 (`FUN_140ab0350`, `set_state(2)`): the builder no longer counts (the caller
    /// takes it out of the foundation's builders when it did), its `MoveTo` goes and stops the
    /// unit, the foundation goes (the caller releases it) and the point; state 2.
    #[test]
    fn a_stop_halts_the_build_and_lets_its_foundation_go() {
        let mut mover = crate::hunt::tests::walker();
        let mut b = BuildAction {
            state: MOVE,
            move_to: Some(MoveTo::Moving),
            goal: Some([3.0, 4.0, 0.0]),
            added: true,
            ..BuildAction::start(31148, [0.0; 3], 0.0, TASK)
        };
        assert_eq!(b.halt(Some(&mut mover)), Ok((true, 31148)));
        assert_eq!(
            (b.state, b.added, b.move_to, b.goal, b.target),
            (HALTED, false, None, None, -1)
        );
        assert_eq!((b.point, b.walk_target), ([-1.0; 3], -1));
        assert_eq!(mover.velocity.unwrap().velocity[2], 0.0);
        // one not counted among the builders, standing: nothing to stop
        let mut idle = BuildAction {
            state: WORK,
            move_to: None,
            ..BuildAction::start(31150, [0.0; 3], 0.0, TASK)
        };
        assert_eq!(idle.halt(None), Ok((false, 31150)));
    }

    /// The Build's constructor (`FUN_140aad4e0`) and `set_state(4)` take the foundation's place
    /// as the action's point (`+0x38`..`+0x40`) and its `MoveTo` the foundation as its target.
    #[test]
    fn a_build_s_point_is_its_foundation_s_place() {
        let b = BuildAction::start(10913, [61.5, 92.5, 1.0], 0.8, TASK);
        assert_eq!(
            (b.point, b.walk_target, b.move_to),
            ([61.5, 92.5, 1.0], 10913, Some(MoveTo::Start))
        );
    }

    /// 428326873's builders 8774 and 8775 (player 6) walking to the foundation 10913 that its
    /// owner deletes in update 4626 (state 7, still in the world): the `MoveTo`'s update finds
    /// its target above state 6 and returns 5 (quiet); the Build lets the foundation go (slot
    /// 0x68 with none: its builder count, the release) and walks to its point (`set_state(11)`:
    /// the old `MoveTo` stops the unit, a quiet `MoveTo` to the point starts in state 3). At
    /// the point (its `MoveTo` arrived, 1) or failed (3), or without one (1), it waits
    /// (`set_state(3)`: notify 0x202 and the standing sprite without queued orders).
    #[test]
    fn a_builder_whose_foundation_dies_walks_to_its_point_then_waits() {
        fn builder(waits: &mut u8, state: Option<u8>) -> Builder<'_> {
            Builder {
                class: VILLAGER,
                master: 83,
                held: 0.0,
                held_kind: -1,
                drop_site: false,
                ai: None,
                waits,
                build_targets: &[],
                look: Looks(&|_| None),
                walk_target_state: state,
            }
        }
        fn walk<'a>(
            mover: &'a mut crate::movement::Mover,
            world: &'a mut Spatial,
            me: Me,
        ) -> Walk<'a> {
            Walk {
                mover: Some(mover),
                me,
                dt: 0.02,
                world,
                plan: None,
            }
        }
        use crate::spatial::Placed;
        let mut world = Spatial::default();
        world.insert(
            10913,
            Placed {
                position: [61.5, 92.5, 0.0],
                radius: [1.0, 1.0],
                class: 3,
                player: 6,
                master: 70,
                obstruction_type: 2,
                buildable_on: 0,
                tall: true,
                speed: 0.0,
                unknown: None,
                obstruction: None,
            },
        );
        let mut mover = crate::hunt::tests::walker();
        let mut c = Construction {
            progress: 3.0,
            desolid: false,
            builders: 1,
            hit_points: 25,
            build_time: 25,
            record: Obstruction { kind: 2, class: 2 },
            snow: false,
            sprite_override: false,
            started: false,
        };
        let mut life = Lifecycle {
            state: 7,
            ..Lifecycle::default()
        };
        let mut b = BuildAction {
            move_to: Some(MoveTo::Moving),
            added: true,
            ..BuildAction::start(10913, [61.5, 92.5, 0.0], 0.8, TASK)
        };
        let site = Site {
            id: 10913,
            life: &mut life,
            construction: &mut c,
            mover: None,
            firepower: None,
            roo_called: None,
            store: None,
            completion: Completion {
                master: None,
                annexed: false,
                attribute_204: false,
                mode_680: 0,
                farm_bonus: [0.0, -1.0, -1.0],
                masters: None,
                computer: false,
                ai: false,
                main: false,
                now: 0,
                rice: false,
            },
            chain: WorkChain::default(),
            main_builders: None,
        };
        let me = Me {
            id: 8774,
            position: [58.0, 90.0, 0.0],
            group: -1,
            action_state: MOVE,
        };
        let mut waits = 0;
        let step = b
            .update(
                walk(&mut mover, &mut world, me),
                Some(site),
                builder(&mut waits, Some(7)),
            )
            .unwrap();
        assert!(step.released && !step.notified);
        assert_eq!(c.builders, 0);
        assert_eq!(
            (b.state, b.target, b.walk_target, b.added),
            (WALK_POINT, -1, -1, false)
        );
        assert_eq!(
            (b.move_to, b.goal, b.point),
            (
                Some(MoveTo::Start),
                Some([61.5, 92.5, 0.0]),
                [61.5, 92.5, 0.0]
            )
        );
        assert_eq!(mover.velocity.unwrap().velocity[2], 0.0);
        // a foundation the world removed (425984813's 4045, built and replaced by its stack
        // unit) is no longer the MoveTo's target, which walks on: no walk to the point
        let mut gone = BuildAction {
            target: -1,
            move_to: Some(MoveTo::Arrived),
            ..BuildAction::start(10913, [61.5, 92.5, 0.0], 0.8, TASK)
        };
        let _ = gone
            .update(
                walk(&mut mover, &mut world, me),
                None,
                builder(&mut waits, None),
            )
            .unwrap();
        assert_eq!(gone.walk_target, -1);
        assert_ne!(gone.state, WALK_POINT);
        // a unit without speed: its MoveTo fails at the start (13)
        let mut still = crate::hunt::tests::walker();
        still.stats.speed = 0.0;
        let mut held = BuildAction {
            target: -1,
            move_to: Some(MoveTo::Moving),
            ..BuildAction::start(10913, [61.5, 92.5, 0.0], 0.8, TASK)
        };
        let _ = held
            .update(
                walk(&mut still, &mut world, me),
                None,
                builder(&mut waits, Some(7)),
            )
            .unwrap();
        assert_eq!(held.move_to, Some(MoveTo::Failed));
        // at the point, failed, or without a MoveTo: set_state(3)
        for end in [Some(MoveTo::Arrived), Some(MoveTo::Failed), None] {
            let mut w = BuildAction {
                state: WALK_POINT,
                target: -1,
                walk_target: -1,
                move_to: end,
                ..BuildAction::start(10913, [61.5, 92.5, 0.0], 0.8, TASK)
            };
            let step = w
                .update(
                    walk(&mut mover, &mut world, me),
                    None,
                    builder(&mut waits, None),
                )
                .unwrap();
            assert_eq!((w.state, w.move_to), (IDLE, None));
            assert!(step.notified && step.stands);
        }
    }

    #[test]
    fn a_builder_sent_to_another_foundation_starts_over() {
        let mut b = BuildAction {
            state: WORK,
            move_to: None,
            added: true,
            timer: 0.5,
            ..BuildAction::start(31148, [0.0; 3], 0.0, TASK)
        };
        b.retarget((31150, [0.0; 3]), 0.0, None).unwrap();
        assert_eq!(b, BuildAction::start(31150, [0.0; 3], 0.0, TASK));
        assert_eq!(
            (b.state, b.added, b.move_to),
            (MOVE, false, Some(MoveTo::Arrived))
        );
    }

    /// records/425906646's update 673: builders 27232 and 27234, at work on house 31011 (state
    /// 7, counted among its builders), are told to build it again: each leaves the builders and
    /// starts over (state 4, its `MoveTo` arrived beside the house); one at work (state 6) stays.
    #[test]
    fn a_builder_told_to_build_its_foundation_again_starts_over() {
        let mut c = Construction {
            progress: 3.0,
            desolid: false,
            builders: 2,
            hit_points: 25,
            build_time: 25,
            record: Obstruction { kind: 2, class: 2 },
            snow: false,
            sprite_override: false,
            started: false,
        };
        let mut b = BuildAction {
            state: PLACE,
            move_to: None,
            added: true,
            ..BuildAction::start(31011, [0.0; 3], 0.0, TASK)
        };
        b.retake(([0.0; 3], 0.0), None, Some(&mut c.builders), false)
            .unwrap();
        assert_eq!(c.builders, 1);
        assert_eq!(b, BuildAction::start(31011, [0.0; 3], 0.0, TASK));
        assert_eq!(b.move_to, Some(MoveTo::Arrived));
        let mut working = BuildAction {
            state: WORK,
            move_to: None,
            ..BuildAction::start(31011, [0.0; 3], 0.0, TASK)
        };
        let before = working;
        working
            .retake(([0.0; 3], 0.0), None, Some(&mut c.builders), false)
            .unwrap();
        assert_eq!(working, before);
        assert_eq!(c.builders, 1);
    }

    /// MarvinBlue/426048731's house 10026 in update 1115 (player 1): the work that reaches 25 s
    /// completes it: its build points hold the build time, its storage (population headroom, 4,
    /// of mode 4) and attributes 98 and 165 go to its owner, it stands in state 2 and its
    /// foundation (27) covers its snow with terrain 6 under layer 32.
    #[test]
    fn the_work_that_reaches_the_build_time_completes_the_building() {
        use crate::spatial::Tiles;
        use crate::terrain::{Terrain, TerrainKind};
        let mut world = Spatial::with_tiles([4, 4], Tiles::new(4, 4, vec![4; 16]));
        world.insert(
            10026,
            crate::spatial::Placed {
                position: [2.0, 2.0, 0.0],
                ..house()
            },
        );
        let mut kinds = vec![
            TerrainKind {
                shown: true,
                flags: 0x20,
                forest: false,
            };
            40
        ];
        kinds[32].flags = 0xa0;
        world.terrain = Some(Terrain::new(
            4,
            4,
            vec![(32, 255, -1); 16],
            kinds,
            vec![vec![0.0; 40]; 5],
        ));
        let master = Master {
            id: 70,
            kind: 80,
            class: 3,
            clearance: [1.0, 1.0],
            terrain_bonus: 4,
            storage: vec![(4, 5.0, 4), (-1, 0.0, 0), (-1, 0.0, 0)],
            // a house's cost: 25 wood paid; the unpaid entry does not count
            cost: vec![(1, 25, true), (4, 5, false)],
            building: Some(crate::masters::BuildingMaster {
                adjacent_mode: 0,
                disappears_when_built: false,
                stack_unit: -1,
                foundation_terrain: 27,
                tech: -1,
                garrison_type: 0,
                garrison_heal_rate: 0.0,
                garrison_repair_rate: 0.0,
                annexes: [-1; 4],
                annex_offsets: [[0.0; 2]; 4],
                head_unit: -1,
                transform_unit: -1,
                destruction_sprite: -1,
            }),
            ..Master::default()
        };
        let mut life = Lifecycle {
            state: 0,
            hp: 548.0,
            master_id: 70,
            ..Lifecycle::default()
        };
        let mut c = Construction {
            progress: 24.99,
            desolid: false,
            builders: 2,
            hit_points: 550,
            build_time: 25,
            record: Obstruction { kind: 2, class: 2 },
            snow: false,
            sprite_override: false,
            started: false,
        };
        let mut site = Site {
            id: 10026,
            life: &mut life,
            construction: &mut c,
            mover: None,
            firepower: None,
            roo_called: None,
            store: None,
            completion: Completion {
                master: Some(&master),
                annexed: false,
                attribute_204: false,
                mode_680: 0,
                farm_bonus: [0.0, -1.0, -1.0],
                masters: None,
                computer: false,
                ai: false,
                main: false,
                now: 0,
                rice: false,
            },
            chain: WorkChain::default(),
            main_builders: None,
        };
        let worked = Construction::work(&mut site, 0.02, &mut world, &mut Pathing::default(), 0);
        assert_eq!(
            worked,
            Ok(Worked::Completed(Built {
                attributes: vec![(4, 5.0), (98, 25.0), (165, 25.0)],
                history: None,
                tech: None,
                cart: None,
                stack: None,
                vanishes: false,
                started: false,
            }))
        );
        assert_eq!((life.state, c.progress, c.snow), (2, 25.0, true));
        let t = world.terrain.as_ref().unwrap();
        assert_eq!((t.kinds[5], t.layers[5], t.kinds[0]), (6, 32, 32));
    }

    /// `FUN_1411dbb70`'s tail (`0x1411dd094`..): the castle (82) and the wonder (276) record
    /// their building in their owner's history (events 6 and 4, [`Built::history`]), count in
    /// attributes 134 and 173, 42 and 174, and add their paid cost to 184 rather than to 98 and
    /// 165, an annex's too; the monastery (104) counts in 52 and adds its paid cost to 98 and
    /// 165 as other buildings do, but as an annex.
    #[test]
    fn the_castle_the_wonder_and_the_monastery_count_their_completion() {
        use crate::history::{CASTLE_BUILT, WONDER_BUILT};
        use crate::spatial::Tiles;
        use crate::terrain::{Terrain, TerrainKind};
        fn complete(id: i16, cost: Vec<(i16, i16, bool)>, main: bool) -> Built {
            let mut world = Spatial::with_tiles([4, 4], Tiles::new(4, 4, vec![4; 16]));
            world.insert(
                10026,
                crate::spatial::Placed {
                    position: [2.0, 2.0, 0.0],
                    master: id,
                    ..house()
                },
            );
            let kind = TerrainKind {
                shown: true,
                flags: 0x20,
                forest: false,
            };
            world.terrain = Some(Terrain::new(
                4,
                4,
                vec![(0, 255, -1); 16],
                vec![kind; 40],
                vec![vec![0.0; 40]; 5],
            ));
            let master = Master {
                id,
                kind: 80,
                class: 3,
                cost,
                building: Some(crate::masters::BuildingMaster {
                    adjacent_mode: 0,
                    disappears_when_built: false,
                    stack_unit: -1,
                    foundation_terrain: -1,
                    tech: -1,
                    garrison_type: 0,
                    garrison_heal_rate: 0.0,
                    garrison_repair_rate: 0.0,
                    annexes: [-1; 4],
                    annex_offsets: [[0.0; 2]; 4],
                    head_unit: -1,
                    transform_unit: -1,
                    destruction_sprite: -1,
                }),
                ..Master::default()
            };
            let mut life = Lifecycle {
                state: 0,
                hp: 100.0,
                master_id: id,
                ..Lifecycle::default()
            };
            let mut c = Construction {
                progress: 100.0,
                desolid: false,
                builders: 1,
                hit_points: 100,
                build_time: 100,
                record: Obstruction { kind: 2, class: 2 },
                snow: false,
                sprite_override: false,
                started: false,
            };
            let mut site = Site {
                id: 10026,
                life: &mut life,
                construction: &mut c,
                mover: None,
                firepower: None,
                roo_called: None,
                store: None,
                completion: Completion {
                    master: Some(&master),
                    annexed: false,
                    attribute_204: false,
                    mode_680: 0,
                    farm_bonus: [0.0, -1.0, -1.0],
                    masters: None,
                    computer: false,
                    ai: false,
                    main,
                    now: 0,
                    rice: false,
                },
                chain: WorkChain::default(),
                main_builders: None,
            };
            let built = Construction::complete(&mut site, &mut world, &mut Pathing::default());
            assert_eq!(life.state, 2);
            built.unwrap()
        }
        let castle = || vec![(2, 650, true)];
        let wonder = || vec![(0, 1000, true), (1, 1000, true), (2, 1000, false)];
        let built = |attributes, history| Built {
            attributes,
            history,
            ..Built::default()
        };
        for main in [false, true] {
            assert_eq!(
                complete(82, castle(), main),
                built(
                    vec![(134, 1.0), (173, 1.0), (184, 650.0)],
                    Some(CASTLE_BUILT)
                )
            );
            assert_eq!(
                complete(276, wonder(), main),
                built(
                    vec![(42, 1.0), (174, 1.0), (184, 2000.0)],
                    Some(WONDER_BUILT)
                )
            );
        }
        assert_eq!(
            complete(104, vec![(1, 175, true)], false),
            built(vec![(52, 1.0), (98, 175.0), (165, 175.0)], None)
        );
        assert_eq!(
            complete(104, vec![(1, 175, true)], true),
            built(vec![(52, 1.0)], None)
        );
    }

    /// MarvinBlue/426048731's house 10028 in update 731: 9177's first work starts it (its record
    /// marks its tiles) and adds 0.021 s of work: 550 hit points over 25 s, so 1.462 hit points.
    #[test]
    fn the_first_work_starts_the_foundation_and_adds_its_share_of_hit_points() {
        use crate::spatial::Tiles;
        let mut world = Spatial::with_tiles([120, 120], Tiles::new(120, 120, vec![4; 120 * 120]));
        world.insert(10028, house());
        let mut life = Lifecycle {
            state: 0,
            hp: 1.0,
            master_id: 70,
            ..Lifecycle::default()
        };
        let mut c = Construction {
            progress: 0.0,
            desolid: true,
            builders: 1,
            hit_points: 550,
            build_time: 25,
            record: Obstruction { kind: 2, class: 2 },
            snow: false,
            sprite_override: false,
            started: false,
        };
        let mut pathing = Pathing::default();
        let mut site = Site {
            id: 10028,
            life: &mut life,
            construction: &mut c,
            mover: None,
            firepower: None,
            roo_called: None,
            store: None,
            completion: Completion {
                master: None,
                annexed: false,
                attribute_204: false,
                mode_680: 0,
                farm_bonus: [0.0, -1.0, -1.0],
                masters: None,
                computer: false,
                ai: false,
                main: false,
                now: 0,
                rice: false,
            },
            chain: WorkChain::default(),
            main_builders: None,
        };
        let rate = 1.0f32 * 1.0 * 0.021;
        assert_eq!(
            Construction::work(&mut site, rate, &mut world, &mut pathing, 0),
            Ok(Worked::Going)
        );
        assert_eq!(life.hp, 1.462);
        assert!(!c.desolid);
        assert_eq!(c.progress, f32::from_bits(STARTED) + rate);
        assert!(world.get(10028).unwrap().obstruction.is_some());
        assert!(world.tiles().word(29, 87).unwrap() & crate::spatial::BLOCKED != 0);
    }

    /// A gate's foundation (master 665) with its two posts (1192, annexes of a building type):
    /// `FUN_1411cc430` starts each desolid post first (its record, its missiles per volley, the
    /// clearing after the update), a post already started stays as it is, then the gate's own
    /// first work starts it.
    #[test]
    fn the_work_starts_the_desolid_annexes_first() {
        use crate::spatial::Tiles;
        let mut world = Spatial::with_tiles([120, 120], Tiles::new(120, 120, vec![4; 120 * 120]));
        let at = |x: f32, id: i16| crate::spatial::Placed {
            position: [x, 88.0, 0.0],
            master: id,
            ..house()
        };
        world.insert(10028, at(30.0, 665));
        world.insert(10029, at(28.5, 1192));
        world.insert(10030, at(31.5, 1192));
        let foundation = |desolid| Construction {
            progress: 0.0,
            desolid,
            builders: 1,
            hit_points: 550,
            build_time: 25,
            record: Obstruction { kind: 2, class: 2 },
            snow: false,
            sprite_override: false,
            started: false,
        };
        let (mut post, mut started_post) = (foundation(true), foundation(false));
        let (mut post_fire, mut started_fire) = (0.0, 0.0);
        let mut life = Lifecycle {
            state: 0,
            hp: 1.0,
            master_id: 665,
            ..Lifecycle::default()
        };
        let mut c = foundation(true);
        let mut pathing = Pathing::default();
        let mut site = Site {
            id: 10028,
            life: &mut life,
            construction: &mut c,
            mover: None,
            firepower: None,
            roo_called: None,
            store: None,
            completion: Completion {
                master: None,
                annexed: true,
                attribute_204: false,
                mode_680: 0,
                farm_bonus: [0.0, -1.0, -1.0],
                masters: None,
                computer: false,
                ai: false,
                main: false,
                now: 0,
                rice: false,
            },
            chain: WorkChain {
                annexes: vec![
                    AnnexStart {
                        id: 10029,
                        construction: &mut post,
                        mover: None,
                        firepower: &mut post_fire,
                        volley: 2.0,
                    },
                    AnnexStart {
                        id: 10030,
                        construction: &mut started_post,
                        mover: None,
                        firepower: &mut started_fire,
                        volley: 2.0,
                    },
                ],
                main: None,
            },
            main_builders: None,
        };
        assert_eq!(
            Construction::work(&mut site, 0.021, &mut world, &mut pathing, 0),
            Ok(Worked::Going)
        );
        assert_eq!(world.take_started(), vec![10029, 10028]);
        assert!(!post.desolid && post.started);
        assert_eq!(post.progress, f32::from_bits(STARTED));
        assert_eq!(post_fire, 2.0);
        assert!(world.get(10029).unwrap().obstruction.is_some());
        assert_eq!((started_post, started_fire), (foundation(false), 0.0));
        assert!(!c.desolid);
    }

    /// An annex (a gate's post) whose main building is in state 0 with a hit point gives it the
    /// work (`FUN_1411cc430`'s loop): the post itself is not worked, and the result is the main
    /// building's (its start's build points, then the work): 0 while it goes on, 1 once it
    /// reaches its build time.
    #[test]
    fn an_annex_s_work_goes_to_its_main_building() {
        let mut world = Spatial::default();
        let foundation = Construction {
            progress: 0.0,
            desolid: true,
            builders: 1,
            hit_points: 550,
            build_time: 25,
            record: Obstruction { kind: 2, class: 2 },
            snow: false,
            sprite_override: false,
            started: false,
        };
        let mut run = |main: Construction, amount: f32| {
            let mut life = Lifecycle {
                state: 0,
                hp: 1.0,
                master_id: 1192,
                ..Lifecycle::default()
            };
            let mut c = foundation;
            let mut site = Site {
                id: 10029,
                life: &mut life,
                construction: &mut c,
                mover: None,
                firepower: None,
                roo_called: None,
                store: None,
                completion: Completion {
                    master: None,
                    annexed: false,
                    attribute_204: false,
                    mode_680: 0,
                    farm_bonus: [0.0, -1.0, -1.0],
                    masters: None,
                    computer: false,
                    ai: false,
                    main: true,
                    now: 0,
                    rice: false,
                },
                chain: WorkChain {
                    annexes: Vec::new(),
                    main: Some(MainWork {
                        construction: main,
                        hp: 1.0,
                    }),
                },
                main_builders: None,
            };
            let worked =
                Construction::work(&mut site, amount, &mut world, &mut Pathing::default(), 0);
            assert_eq!(
                (c, life.hp),
                (foundation, 1.0),
                "the post itself is not worked"
            );
            worked
        };
        assert_eq!(
            run(foundation, 0.021),
            Ok(Worked::Main {
                amount: 0.021,
                going: true
            })
        );
        let nearly = Construction {
            progress: 24.99,
            desolid: false,
            ..foundation
        };
        assert_eq!(
            run(nearly, 0.021),
            Ok(Worked::Main {
                amount: 0.021,
                going: false
            })
        );
    }

    use crate::movement::Me;
    use crate::spatial::Spatial;

    /// MarvinBlue/426048731 after tick 679: builder 9175 stands on the house foundation 10028's
    /// footprint, works on it (state 6) and walks off it in update 681 (state 223) to the free tile
    /// the game picks, (29.5, 89.5).
    #[test]
    fn a_builder_on_its_footprint_faces_it_then_walks_to_a_free_tile_beside_it() {
        use crate::movement::{MoveStats, Mover};
        use crate::spatial::{Placed, Tiles};
        let mut world = Spatial::with_tiles([40, 120], Tiles::new(40, 120, vec![4; 40 * 120]));
        let unit = |x, y, radius, master, kind| Placed {
            position: [x, y, 0.0],
            radius: [radius, radius],
            class: 4,
            player: 2,
            master,
            obstruction_type: kind,
            buildable_on: 0,
            tall: true,
            speed: 0.0,
            unknown: None,
            obstruction: None,
        };
        world.insert(9175, unit(30.0, 89.0, 0.2, 118, 5));
        world.insert(10028, unit(30.0, 88.0, 1.0, 70, 2));
        let mut mover = Mover {
            forward: [1.0, 0.0, 0.0],
            right: [0.0, 0.0, -1.0],
            velocity: None,
            paths: vec![],
            factor: 1.0,
            stats: MoveStats {
                fly_mode: 0,
                speed: 0.8,
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
                walk_speed: 1.0,
                stand_speed: 1.0,
                run_speed: 1.0,
                facets: Some(crate::facing::SpriteFacets {
                    count: 16,
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
        };
        let mut build = BuildAction {
            target: 10028,
            state: 6,
            move_to: None,
            goal: None,
            walk_target: -1,
            point: [-1.0; 3],
            task: TASK,
            added: false,
            timer: 0.0,
        };
        let mut life = Lifecycle {
            state: 0,
            ..Lifecycle::default()
        };
        let mut construction = Construction {
            progress: 0.0,
            desolid: true,
            builders: 0,
            hit_points: 550,
            build_time: 25,
            record: Obstruction { kind: 2, class: 2 },
            snow: false,
            sprite_override: false,
            started: false,
        };
        let site = Site {
            id: 10028,
            life: &mut life,
            construction: &mut construction,
            mover: None,
            firepower: None,
            roo_called: None,
            store: None,
            completion: Completion {
                master: None,
                annexed: false,
                attribute_204: false,
                mode_680: 0,
                farm_bonus: [0.0, -1.0, -1.0],
                masters: None,
                computer: false,
                ai: false,
                main: false,
                now: 0,
                rice: false,
            },
            chain: WorkChain::default(),
            main_builders: None,
        };
        let _ = build
            .update(
                Walk {
                    mover: Some(&mut mover),
                    me: Me {
                        id: 9175,
                        position: [30.0, 89.0, 0.0],
                        group: -1,
                        action_state: 0,
                    },
                    dt: 0.02,
                    world: &mut world,
                    plan: None,
                },
                Some(site),
                Builder {
                    class: VILLAGER,
                    master: 83,
                    held: 0.0,
                    held_kind: -1,
                    drop_site: false,
                    ai: None,
                    waits: &mut 0,
                    build_targets: &[],
                    look: Looks(&|_| None),
                    walk_target_state: Some(0),
                },
            )
            .unwrap();
        assert_eq!(
            (build.state, build.move_to, build.goal),
            (223, Some(MoveTo::Start), Some([29.5, 89.5, 0.0]))
        );
    }

    /// census v5's 240 stops "a foundation clears objects off its footprint": the Build's
    /// `set_state(7)` on a desolid foundation clears its footprint with `param_6`
    /// (`FUN_1415c3620`): a stump (a tree of at most 1 hit point) on it loses its record at once
    /// (`FUN_141189670`), its delete is the caller's; the occupants' test passes over it (state
    /// 7) and the tiles hold no other player's mark: the builder joins the work.
    #[test]
    fn a_builder_s_set_state_7_clears_a_stump_and_joins() {
        use crate::spatial::{Placed, Tiles};
        let mut world = Spatial::with_tiles([20, 20], Tiles::new(20, 20, vec![0; 400]));
        world.insert(
            10028,
            Placed {
                position: [10.5, 10.5, 0.0],
                player: 1,
                ..house()
            },
        );
        world.insert(
            77,
            Placed {
                position: [10.2, 10.2, 0.0],
                radius: [0.5, 0.5],
                class: TREE,
                player: 0,
                master: 349,
                obstruction_type: 2,
                buildable_on: 0,
                tall: true,
                speed: 0.0,
                unknown: None,
                obstruction: Some(Obstruction { kind: 2, class: 0 }),
            },
        );
        assert_ne!(world.tiles().word(10, 10).unwrap() >> 23, 0);
        let looks = |id: i32| {
            (id == 77).then_some(ClearLook {
                hp: 0.5,
                height: 1.0,
                roo_called: false,
                kind: 10,
                state: 2,
                desolid: false,
            })
        };
        let mut life = Lifecycle {
            state: 0,
            master_id: 70,
            ..Lifecycle::default()
        };
        let mut construction = Construction {
            progress: 0.0,
            desolid: true,
            builders: 0,
            hit_points: 550,
            build_time: 25,
            record: Obstruction { kind: 2, class: 2 },
            snow: false,
            sprite_override: false,
            started: false,
        };
        let site = Site {
            id: 10028,
            life: &mut life,
            construction: &mut construction,
            mover: None,
            firepower: None,
            roo_called: None,
            store: None,
            completion: Completion {
                master: None,
                annexed: false,
                attribute_204: false,
                mode_680: 0,
                farm_bonus: [0.0, -1.0, -1.0],
                masters: None,
                computer: false,
                ai: false,
                main: false,
                now: 0,
                rice: false,
            },
            chain: WorkChain::default(),
            main_builders: None,
        };
        let mut build = BuildAction {
            state: WAIT,
            move_to: None,
            timer: 0.01,
            ..BuildAction::start(10028, [0.0; 3], 0.0, TASK)
        };
        let step = build
            .update(
                Walk {
                    mover: None,
                    me: Me {
                        id: 5,
                        position: [12.5, 10.5, 0.0],
                        group: -1,
                        action_state: 0,
                    },
                    dt: 0.02,
                    world: &mut world,
                    plan: None,
                },
                Some(site),
                Builder {
                    class: VILLAGER,
                    master: 83,
                    held: 0.0,
                    held_kind: -1,
                    drop_site: false,
                    ai: None,
                    waits: &mut 0,
                    build_targets: &[],
                    look: Looks(&looks),
                    walk_target_state: Some(0),
                },
            )
            .unwrap();
        assert_eq!(
            step.cleared,
            [Cleared {
                id: 77,
                wall: false,
                record_goes: true,
            }]
        );
        assert_eq!(step.occupants, Vec::<i32>::new());
        assert_eq!(world.get(77).unwrap().obstruction, None);
        assert_eq!(world.tiles().word(10, 10).unwrap() >> 23, 0);
        assert_eq!(
            (build.state, build.added, construction.builders),
            (PLACE, true, 1)
        );
    }

    /// The update's case 1 and case 0xc (`0x140ab0370`): a Build its unit's stop halted (state
    /// 2) fails (`set_state(13)`, as 1: the unit takes its standing sprite,
    /// [`BuildStep::stands`]) and the update returns 0; failed, it posts notify 0x1f9 with its
    /// action value to its unit AI ([`BuildStep::failed`]), halts (`set_state(2)`: the unit
    /// stands, a target goes, [`BuildStep::released`]) and returns 3, which removes it
    /// ([`BuildStep::done`]). In state 3 with a target the update does nothing (the default
    /// case).
    #[test]
    fn a_halted_build_fails_and_its_failure_removes_it() {
        fn update(b: &mut BuildAction, world: &mut Spatial) -> BuildStep {
            let mut ai = UnitAi::default();
            let walk = Walk {
                mover: None,
                me: Me {
                    id: 9,
                    position: [0.0; 3],
                    group: -1,
                    action_state: 0,
                },
                dt: 0.02,
                world,
                plan: None,
            };
            let builder = Builder {
                class: VILLAGER,
                master: 83,
                held: 0.0,
                held_kind: -1,
                drop_site: false,
                ai: Some(&mut ai),
                waits: &mut 0,
                build_targets: &[],
                look: Looks(&|_| None),
                walk_target_state: Some(0),
            };
            b.update(walk, None, builder).unwrap()
        }
        let mut world = Spatial::default();
        world.insert(10028, house());
        let mut b = BuildAction {
            state: HALTED,
            move_to: None,
            target: -1,
            ..BuildAction::start(10028, [0.0; 3], 0.0, TASK)
        };
        let step = update(&mut b, &mut world);
        assert_eq!(b.state, FAILED);
        assert_eq!(
            step,
            BuildStep {
                stands: true,
                ..BuildStep::default()
            }
        );
        let step = update(&mut b, &mut world);
        assert_eq!((b.state, b.target), (HALTED, -1));
        assert_eq!(
            step,
            BuildStep {
                stands: true,
                failed: true,
                done: true,
                ..BuildStep::default()
            }
        );
        // failed with its foundation: the halt lets it go
        let mut holding = BuildAction {
            state: FAILED,
            move_to: None,
            ..BuildAction::start(10028, [0.0; 3], 0.0, TASK)
        };
        let step = update(&mut holding, &mut world);
        assert_eq!((holding.state, holding.target), (HALTED, -1));
        assert!(step.released && step.failed && step.done);
        // waiting with a target: nothing
        let mut waiting = BuildAction {
            state: IDLE,
            move_to: None,
            ..BuildAction::start(10028, [0.0; 3], 0.0, TASK)
        };
        let before = waiting;
        let step = update(&mut waiting, &mut world);
        assert_eq!((waiting, step), (before, BuildStep::default()));
    }

    /// census v5's 217 stops "a dying foundation" went on to "a Build whose foundation died":
    /// a builder at work (state 7, among the builders) on a foundation in state 3
    /// (`0x140ab0370`'s prelude): `set_state(4)` toward it (out of its builders, a `MoveTo`
    /// arrived beside it), slot 0x68 with none (the caller releases it); the `MoveTo` is there,
    /// so `set_state(6)` without a target turns to `set_state(3)`: without a unit AI it tells
    /// itself 0x202. Master 621 and the gates (class 39) end their work instead once built
    /// (`set_state(224)`), a foundation in state 1 or 2 too.
    #[test]
    fn a_builder_lets_a_dead_foundation_go() {
        use crate::movement::{MoveStats, Mover};
        let mover = || Mover {
            forward: [1.0, 0.0, 0.0],
            right: [0.0, 0.0, -1.0],
            velocity: None,
            paths: vec![],
            factor: 1.0,
            stats: MoveStats {
                fly_mode: 0,
                speed: 0.8,
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
                walk_speed: 1.0,
                stand_speed: 1.0,
                run_speed: 1.0,
                facets: None,
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
        };
        let run = |master: Master, state: u8, progress: f32| {
            let mut world = Spatial::default();
            world.insert(10028, house());
            let mut life = Lifecycle {
                state,
                master_id: master.id,
                ..Lifecycle::default()
            };
            let mut c = Construction {
                progress,
                desolid: false,
                builders: 2,
                hit_points: 550,
                build_time: 25,
                record: Obstruction { kind: 2, class: 2 },
                snow: false,
                sprite_override: false,
                started: false,
            };
            let mut b = BuildAction {
                state: PLACE,
                move_to: None,
                added: true,
                ..BuildAction::start(10028, [0.0; 3], 0.0, TASK)
            };
            let mut m = mover();
            let mut ai = UnitAi::default();
            let step = b
                .update(
                    Walk {
                        mover: Some(&mut m),
                        me: Me {
                            id: 9,
                            position: [30.0, 89.1, 0.0],
                            group: -1,
                            action_state: 0,
                        },
                        dt: 0.02,
                        world: &mut world,
                        plan: None,
                    },
                    Some(Site {
                        id: 10028,
                        life: &mut life,
                        construction: &mut c,
                        mover: None,
                        firepower: None,
                        roo_called: None,
                        store: None,
                        completion: Completion {
                            master: Some(&master),
                            annexed: false,
                            attribute_204: false,
                            mode_680: 0,
                            farm_bonus: [0.0, -1.0, -1.0],
                            masters: None,
                            computer: false,
                            ai: false,
                            main: false,
                            now: 0,
                            rice: false,
                        },
                        chain: WorkChain::default(),
                        main_builders: None,
                    }),
                    Builder {
                        class: VILLAGER,
                        master: 83,
                        held: 0.0,
                        held_kind: -1,
                        drop_site: false,
                        ai: Some(&mut ai),
                        waits: &mut 0,
                        build_targets: &[],
                        look: Looks(&|_| None),
                        walk_target_state: Some(0),
                    },
                )
                .unwrap();
            (
                b.state,
                b.target,
                c.builders,
                step.released,
                step.notified || step.search,
            )
        };
        let house_master = Master {
            id: 70,
            class: 3,
            ..Master::default()
        };
        assert_eq!(
            run(house_master.clone(), 3, 10.0),
            (IDLE, -1, 1, true, true)
        );
        // a built one in state 2 ends the work
        assert_eq!(
            run(house_master, 2, 25.0),
            (FINISHED, 10028, 1, false, true)
        );
        // a dead gate (class 39) not built yet lets go, one built ends the work
        let gate = Master {
            id: 1584,
            class: CLASS_39,
            ..Master::default()
        };
        assert_eq!(run(gate.clone(), 3, 10.0), (IDLE, -1, 1, true, true));
        assert_eq!(run(gate, 3, 25.0), (FINISHED, 10028, 1, false, true));
    }

    #[test]
    fn a_builder_at_the_foundation_works_in_its_first_update_the_others_walk_to_it() {
        fn walk(id: i32, world: &mut Spatial) -> Walk<'_> {
            Walk {
                mover: None,
                me: Me {
                    id,
                    position: [0.0; 3],
                    group: -1,
                    action_state: 0,
                },
                dt: 0.02,
                world,
                plan: None,
            }
        }
        let mut near = BuildAction::start(10028, [0.0; 3], 0.0, TASK);
        let mut far = BuildAction::start(10028, [0.0; 3], 0.8, TASK);
        assert_eq!((near.state, near.move_to), (4, Some(MoveTo::Arrived)));
        assert_eq!((far.state, far.move_to), (4, Some(MoveTo::Start)));
        let mut world = Spatial::default();
        let mut waits = [0u8; 3];
        let mut waits = waits.iter_mut();
        let mut builder = || Builder {
            class: VILLAGER,
            master: 83,
            held: 0.0,
            held_kind: -1,
            drop_site: false,
            ai: None,
            waits: waits.next().expect("a wait count per update"),
            build_targets: &[],
            look: Looks(&|_| None),
            walk_target_state: Some(0),
        };
        world.insert(10028, house());
        let _ = near.update(walk(1, &mut world), None, builder()).unwrap();
        assert_eq!((near.state, near.move_to), (6, None));
        // census v5's 208 stops "a Build whose foundation is gone": the far one finds its
        // foundation's id gone from the world (0x140ab0370's prelude): slot 0x68 with none, then
        // set_state(3): its MoveTo goes, and without orders it tells its unit AI (0x202)
        let mut empty = Spatial::default();
        let step = far.update(walk(2, &mut empty), None, builder()).unwrap();
        assert_eq!(
            (far.state, far.target, far.move_to, step.notified, step.done),
            (IDLE, -1, None, true, false)
        );
    }
}

#[cfg(test)]
mod stack_tests {
    use super::*;
    use crate::lifecycle::{Class, Lifecycle};
    use crate::masters::{BuildingMaster, Masters};
    use crate::spatial::{Placed, Tiles};
    use crate::turn::{Player, Turns, Unit};

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
    fn world() -> Turns {
        let ms = masters();
        let foundation = Unit {
            id: 8759,
            position: [54.0, 147.0, 0.0],
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
        let mut t = Turns::new(
            0,
            1,
            0,
            vec![0; 2],
            vec![
                Player::default(),
                Player {
                    objects: vec![foundation],
                    attributes: vec![0.0; 600],
                    ..Player::default()
                },
            ],
        );
        t.spatial = Spatial::with_tiles([200, 200], Tiles::new(200, 200, vec![0; 200 * 200]));
        t.spatial.terrain = Some(crate::terrain::Terrain::new(
            200,
            200,
            vec![(0, 255, -1); 200 * 200],
            vec![
                crate::terrain::TerrainKind {
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
                history: Some(crate::history::CASTLE_BUILT),
                ..Built::default()
            },
        )
        .unwrap();
        assert_eq!(
            t.players[1].history.events,
            [crate::history::Event {
                kind: crate::history::CASTLE_BUILT,
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
        let unit = |id: i32| t.players[1].objects.iter().find(|u| u.id == id).unwrap();
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
