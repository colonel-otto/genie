//! The views a unit's update and its unit AI read the world through ([`UnitView`],
//! [`Context`], [`Foreign`], [`OwnerView`], the pickers).

use crate::masters::Masters;
use crate::math::rng::Well512;
use crate::pathing::Pathing;
use crate::spatial::{Placed, Spatial};
use crate::sprite::Graphic;
use crate::tasks::DropSite;
use crate::turn::player::Player;
use crate::turn::unit::Unit;
use crate::turn::world::Releaser;

mod commands;
mod context;
mod side;
mod unit_view;
mod world_picker;

pub(in crate::turn) use self::commands::town_centre;
pub(in crate::turn) use self::context::{Context, ObjectOutputs};
use self::world_picker::NOT_HELD;
#[cfg(test)]
use self::world_picker::attacks;
pub(in crate::turn) use self::world_picker::{WorldPicker, patrol_information};
#[cfg(test)]
use crate::unit_ai::UnitAi;

/// The villagers a villager's attack command switches to by its task swap group (its player's
/// master table's entries at `+0x298` and `+0x928`).
const VILLAGER_MALE: i16 = 83;
const VILLAGER_FEMALE: i16 = 293;

/// What a tick's update of one object needs from the world.
/// What a hunt's drop-off ([`Unit::drop_off_in`]) reads besides its unit: the owner's masters,
/// the world with its zone maps and planners, the world's time, the Object stream (the drop-site
/// search's plans), the owner's type and construction rate.
pub(crate) struct DropWorld<'a> {
    pub masters: Option<&'a Masters>,
    pub spatial: &'a Spatial,
    pub zones: &'a mut crate::pathing::zones::Zones,
    pub pathing: &'a mut Pathing,
    pub object: &'a mut Well512,
    pub world_time: u32,
    pub computer: bool,
    pub construction_rate: f32,
    pub groups: &'a crate::group::UnitGroups,
}

/// A unit command a unit AI's update gave that the caller runs after it ([`Context::works`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::turn) enum AiWork {
    /// `HuntObject`'s unit part on the target: the stop, then the hunt command
    /// ([`Turns::hunt_command`]).
    Hunt(i32),
    /// `GatherObject`'s gather command on the target ([`Turns::gather_command`]).
    Gather(i32),
    /// The unit's slot 0x7e8 on an object at a point ([`Turns::order_work`]).
    Work(crate::order::Work),
    /// `BuildObject`'s unit part: the build command on the foundation ([`Turns::build_command`]).
    Build(i32),
    /// The villager's case 0x21c: its load to the drop-site table's building
    /// ([`Turns::drop_elsewhere`]).
    DropElsewhere(i32, Option<i32>),
}

/// What a repair changed of an object of another player than the repairer's
/// ([`Context::repair_writes`]): its hit points and powerups, its repairers (`+0x9e8`) and the
/// armor its charge's hits took, which a full repair gives back (`FUN_1412066a0`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::turn) struct RepairWrite {
    /// The object.
    pub(in crate::turn) id: i32,
    /// Its new hit points and hit point powerups (`+0xa38`, `+0xa3c`).
    pub(in crate::turn) hp: Option<(f32, [f32; 2])>,
    /// What its repairers change by.
    pub(in crate::turn) repairers: i8,
    /// Its armor removed (`+0x9e4`, `+0x9e6`) goes back to 0.
    pub(in crate::turn) restored: bool,
}

/// What the unit's slot 0x388 (`FUN_141226fc0`) found while its unit AI ran
/// ([`crate::unit_ai::UnitSide::wait_solidify`]): the foundation, the task its slot 0x820 found
/// on it and the master the search switches the unit to (slot 0x90; None when its own master's
/// list gave the task). The caller makes them the unit's after the update
/// ([`Turns::wait_for_foundation`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Wait {
    pub(crate) target: i32,
    pub(crate) task: crate::masters::Task,
    pub(crate) switch: Option<i16>,
}

/// The unit's repair command (slot 0x378, `FUN_141217c00`) as its unit AI gave it
/// ([`UnitView::repair_command`]), which the caller makes after the update
/// ([`Turns::repair_order`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum RepairOrder {
    /// The Repair on top takes the target (its slot 0x48, `FUN_140adb480`).
    Retake(i32),
    /// The masters the unit switches to (slot 0x90 with 0): the repairer of its task swap group,
    /// then the one the task search's swap group gave; then a Repair on the task found, if any.
    Start {
        target: i32,
        switches: [Option<i16>; 2],
        task: Option<crate::masters::Task>,
    },
}

/// The unit's enter command (slot 0x3a0, `FUN_1410847e0`, forced) as its unit AI gave it
/// ([`UnitView::enter_command`]), which the caller makes after the update
/// ([`Turns::enter_order`]): the master the task search's swap group gave (slot 0x90 with 0),
/// then the Enter on the task found into the target (an annex: its main building, the Enter's
/// target), under an Artifact on top or not.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct EnterOrder {
    pub(crate) target: i32,
    pub(crate) entered: i32,
    pub(crate) task: crate::masters::Task,
    pub(crate) switch: Option<i16>,
    pub(crate) under_artifact: bool,
}

/// An action the unit AI gave its unit while it ran, made after the update.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum AiStart {
    /// A wait for a foundation ([`Wait`], [`Turns::wait_for_foundation`]).
    Wait(Wait),
    /// A repair ([`RepairOrder`], [`Turns::repair_order`]).
    Repair(RepairOrder),
    /// An enter ([`EnterOrder`], [`Turns::enter_order`]).
    Enter(EnterOrder),
    /// An unload command at a point (slot 0x3a8, [`Turns::transport_order`]).
    Transport([f32; 3]),
}

/// The players but the owner of the unit that updates, read-only: the players before it and
/// after it.
#[derive(Clone, Copy)]
pub(in crate::turn) struct Foreign<'a> {
    pub(in crate::turn) before: &'a [Player],
    pub(in crate::turn) after: &'a [Player],
    /// The live list of the player whose list is out, once its object changed hands: that
    /// object's AI part reads it as another player's ([`OwnerView`]).
    pub(in crate::turn) left: Option<(usize, &'a [Unit])>,
}

impl<'a> Foreign<'a> {
    /// The object `id` in another player's live or sleeping list, with that player.
    pub(in crate::turn) fn find(&self, id: i32) -> Option<(usize, &'a Unit)> {
        let skip = self.before.len() + 1;
        let before = self.before.iter().enumerate();
        let after = self.after.iter().enumerate().map(|(q, p)| (q + skip, p));
        before
            .chain(after)
            .find_map(|(q, p)| {
                p.objects
                    .iter()
                    .chain(&p.sleeping)
                    .find(|u| u.id == id)
                    .map(|u| (q, u))
            })
            .or_else(|| {
                let (q, list) = self.left?;
                list.iter().find(|u| u.id == id).map(|u| (q, u))
            })
    }

    /// Player `q`, unless it is the one whose list is out.
    pub(in crate::turn) fn player(&self, q: usize) -> Option<&'a Player> {
        let own = self.before.len();
        match q.cmp(&own) {
            std::cmp::Ordering::Less => self.before.get(q),
            std::cmp::Ordering::Equal => None,
            std::cmp::Ordering::Greater => self.after.get(q - own - 1),
        }
    }
}

/// What a unit's AI part reads of its owner's side ([`Unit::ai_part`]): the owner, its other
/// objects, the other players' lists, its masters, diplomacy, attributes, civ and construction
/// rate. A unit another player took over in its update reads its new owner's, with its old
/// owner's list among the others' (`RGE_Combat_Object::update` reads the owner after the
/// change).
#[derive(Clone, Copy)]
pub(in crate::turn) struct OwnerView<'a> {
    pub(in crate::turn) player: usize,
    pub(in crate::turn) others: &'a [Unit],
    pub(in crate::turn) foreign: Foreign<'a>,
    pub(in crate::turn) masters: Option<&'a Masters>,
    pub(in crate::turn) relations: &'a [u8],
    pub(in crate::turn) attributes: &'a [f32],
    pub(in crate::turn) civ: u8,
    pub(in crate::turn) construction_rate: f32,
    pub(in crate::turn) ai_settings: crate::turn::player::AiSettings,
}

impl<'a> OwnerView<'a> {
    /// The owner's side: player `new`'s when the unit changed hands to it in its update, else
    /// the updating player's (`c`, its other objects `others`).
    pub(in crate::turn) fn of<'c: 'a>(
        c: &Context<'c>,
        others: &'a [Unit],
        new: Option<usize>,
    ) -> Self {
        let own = Self {
            player: c.player,
            others,
            foreign: c.foreign,
            masters: c.masters,
            relations: c.relations,
            attributes: c.attributes,
            civ: c.civ,
            construction_rate: c.construction_rate,
            ai_settings: c.ai_settings,
        };
        let Some((new, q)) = new.and_then(|new| Some((new, c.foreign.player(new)?))) else {
            return own;
        };
        Self {
            player: new,
            others: &q.objects,
            foreign: Foreign {
                left: Some((c.player, others)),
                ..c.foreign
            },
            masters: c.every_masters.get(new),
            relations: &q.relations,
            attributes: &q.attributes,
            civ: q.civ,
            construction_rate: q.construction_rate,
            ai_settings: q.ai_settings,
        }
    }
}

/// What [`UnitView::hunt_now`] hunts by ([`UnitView::hunt_plan`]): the task, the master the
/// unit switches to (none to keep its own), its own master, the quarry and the target's place.
///
/// [`UnitView::hunt_now`]: crate::turn::view::UnitView::hunt_now
struct HuntPlan<'u> {
    task: crate::masters::Task,
    switch: Option<&'u crate::masters::Master>,
    own: &'u crate::masters::Master,
    quarry: crate::hunt::Quarry,
    them: Placed,
}

/// A unit and the world as its unit AI asks them while it runs, the AI taken out of the unit
/// for that time ([`crate::unit_ai::UnitSide`]): the searches and the unit's own commands.
pub(in crate::turn) struct UnitView<'u> {
    /// The unit, without its unit AI.
    pub(in crate::turn) unit: &'u mut Unit,
    /// What the attack check of a search reads of the unit.
    pub(in crate::turn) attacker: crate::unit_ai::Attacker,
    /// The owner, as the AI runs (a captured unit's new one).
    pub(in crate::turn) player: usize,
    /// The owner is a computer player.
    pub(in crate::turn) computer: bool,
    /// The world's objects (and the players' scout maps, which an auto scout makes and marks).
    pub(in crate::turn) spatial: &'u mut Spatial,
    /// The planners and the mesh, which a reach test walks.
    pub(in crate::turn) pathing: &'u mut Pathing,
    /// The world's unit groups, which a reach test's long path reads.
    pub(in crate::turn) groups: &'u crate::group::UnitGroups,
    /// The Object stream, which a reach test's new path draws from.
    pub(in crate::turn) object: &'u mut Well512,
    /// The Sprite stream, which a new sprite draws from.
    pub(in crate::turn) sprite: &'u mut Well512,
    /// The world's time.
    pub(in crate::turn) world_time: u32,
    /// The owner's construction rate.
    pub(in crate::turn) construction_rate: f32,
    /// The owner's other objects.
    pub(in crate::turn) others: &'u [Unit],
    /// The map's terrain zones, which an explorer's search reads.
    pub(in crate::turn) zones: &'u mut crate::pathing::zones::Zones,
    /// The other players' lists.
    pub(in crate::turn) foreign: Foreign<'u>,
    /// Every player's masters.
    pub(in crate::turn) every: &'u [Masters],
    /// The owner's masters.
    pub(in crate::turn) masters: Option<&'u Masters>,
    /// The sprites.
    pub(in crate::turn) graphics: &'u std::collections::BTreeMap<i16, Graphic>,
    /// The objects an action took (slot 0x178), the caller's.
    pub(in crate::turn) takes: &'u mut Vec<(i32, Releaser)>,
    /// The objects an action let go (slot 0x180), the caller's.
    pub(in crate::turn) releases: &'u mut Vec<(i32, Releaser)>,
    /// The foundations a counted builder's Build left ([`Context::builder_leaves`]), the
    /// caller's.
    pub(in crate::turn) builder_leaves: &'u mut Vec<i32>,
    /// The actions its slots 0x388 and 0x378 gave ([`AiStart`]), the caller's.
    pub(in crate::turn) starts: &'u mut Vec<AiStart>,
    /// What its master change (slot 0x90) gives or takes of its owner's attributes, the caller's.
    pub(in crate::turn) attribute_adds: &'u mut Vec<(i16, f32)>,
    /// Its master change's tracker promotion ([`Context::promotions`]), the caller's.
    pub(in crate::turn) promotions: &'u mut Vec<(i32, i16, i16)>,
    /// What its task search reads of its owner: its diplomacy, attributes and civ, and the drop
    /// sites.
    pub(in crate::turn) relations: &'u [u8],
    pub(in crate::turn) attributes: &'u [f32],
    pub(in crate::turn) civ: u8,
    /// The owner's AI settings.
    pub(in crate::turn) ai_settings: crate::turn::player::AiSettings,
    pub(in crate::turn) drop_sites: &'u [DropSite],
    /// The owner's object tracker, whose done lists the drop-site table's search reads.
    pub(in crate::turn) tracker: &'u crate::tracker::Tracker,
    /// The game's setting at `+0x680` ([`Turns::mode_680`]), which a hit's source carries.
    pub(in crate::turn) mode_680: i32,
    /// The game's carry rule ([`crate::settings::Settings::carry_rule`]), which a hunt's start
    /// reads.
    pub(in crate::turn) carry_rule: bool,
}

#[cfg(test)]
mod tests;
