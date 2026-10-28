//! The views a unit's update and its unit AI read the world through ([`UnitView`],
//! [`Context`], [`Foreign`], [`OwnerView`], the pickers).

use crate::map::spatial::Spatial;
use crate::math::rng::Well512;
use crate::objects::masters::Masters;
use crate::objects::release::Releaser;
use crate::objects::sprite::Graphic;
use crate::objects::unit::Unit;
use crate::pathing::Pathing;
use crate::unit_ai::task_finder::DropSite;
use crate::world::others::OthersRef;
use crate::world::player::Player;

mod commands;
mod side;
mod unit_view;
mod world_picker;

pub(crate) use self::commands::town_centre;
pub(crate) use self::world_picker::NOT_HELD;
#[cfg(test)]
use self::world_picker::attacks;
pub(crate) use self::world_picker::{WorldPicker, patrol_information};
#[cfg(test)]
use crate::unit_ai::UnitAi;
use crate::world::context::Context;

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
    pub groups: &'a crate::groups::UnitGroups,
}

/// A unit command a unit AI's update gave that the caller runs after it ([`Context::works`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum AiWork {
    /// `HuntObject`'s unit part on the target: the stop, then the hunt command
    /// ([`World::hunt_command`]).
    Hunt(i32),
    /// `GatherObject`'s gather command on the target ([`World::gather_command`]).
    Gather(i32),
    /// The unit's slot 0x7e8 on an object at a point ([`World::order_work`]).
    Work(crate::orders::Work),
    /// `BuildObject`'s unit part: the build command on the foundation ([`World::build_command`]).
    Build(i32),
    /// The villager's case 0x21c: its load to the drop-site table's building
    /// ([`World::drop_elsewhere`]).
    DropElsewhere(i32, Option<i32>),
}

/// What a repair changed of an object of another player than the repairer's
/// ([`Context::repair_writes`]): its hit points and powerups, its repairers (`+0x9e8`) and the
/// armor its charge's hits took, which a full repair gives back (`FUN_1412066a0`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RepairWrite {
    /// The object.
    pub(crate) id: i32,
    /// Its new hit points and hit point powerups (`+0xa38`, `+0xa3c`).
    pub(crate) hp: Option<(f32, [f32; 2])>,
    /// What its repairers change by.
    pub(crate) repairers: i8,
    /// Its armor removed (`+0x9e4`, `+0x9e6`) goes back to 0.
    pub(crate) restored: bool,
}

/// What the unit's slot 0x388 (`FUN_141226fc0`) found while its unit AI ran
/// ([`crate::unit_ai::UnitSide::wait_solidify`]): the foundation, the task its slot 0x820 found
/// on it and the master the search switches the unit to (slot 0x90; None when its own master's
/// list gave the task). The caller makes them the unit's after the update
/// ([`World::wait_for_foundation`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Wait {
    pub(crate) target: i32,
    pub(crate) task: crate::objects::masters::Task,
    pub(crate) switch: Option<i16>,
}

/// The unit's repair command (slot 0x378, `FUN_141217c00`) as its unit AI gave it
/// ([`UnitView::repair_command`]), which the caller makes after the update
/// ([`World::repair_order`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum RepairOrder {
    /// The Repair on top takes the target (its slot 0x48, `FUN_140adb480`).
    Retake(i32),
    /// The masters the unit switches to (slot 0x90 with 0): the repairer of its task swap group,
    /// then the one the task search's swap group gave; then a Repair on the task found, if any.
    Start {
        target: i32,
        switches: [Option<i16>; 2],
        task: Option<crate::objects::masters::Task>,
    },
}

/// The unit's enter command (slot 0x3a0, `FUN_1410847e0`, forced) as its unit AI gave it
/// ([`UnitView::enter_command`]), which the caller makes after the update
/// ([`World::enter_order`]): the master the task search's swap group gave (slot 0x90 with 0),
/// then the Enter on the task found into the target (an annex: its main building, the Enter's
/// target), under an Artifact on top or not.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct EnterOrder {
    pub(crate) target: i32,
    pub(crate) entered: i32,
    pub(crate) task: crate::objects::masters::Task,
    pub(crate) switch: Option<i16>,
    pub(crate) under_artifact: bool,
}

/// An action the unit AI gave its unit while it ran, made after the update.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum AiStart {
    /// A wait for a foundation ([`Wait`], [`World::wait_for_foundation`]).
    Wait(Wait),
    /// A repair ([`RepairOrder`], [`World::repair_order`]).
    Repair(RepairOrder),
    /// An enter ([`EnterOrder`], [`World::enter_order`]).
    Enter(EnterOrder),
    /// An unload command at a point (slot 0x3a8, [`World::transport_order`]).
    Transport([f32; 3]),
}

/// The players but the owner of the unit that updates, read-only: the players before it and
/// after it.
#[derive(Clone, Copy)]
pub(crate) struct Foreign<'a> {
    pub(crate) before: &'a [Player],
    pub(crate) after: &'a [Player],
    /// The live list of the player whose list is out, once its object changed hands: that
    /// object's AI part reads it as another player's ([`OthersRef::changed_hands`]).
    pub(crate) left: Option<(usize, &'a crate::objects::object_list::ObjectList)>,
}

impl<'a> Foreign<'a> {
    /// Player `q`, unless it is the one whose list is out.
    pub(crate) fn player(&self, q: usize) -> Option<&'a Player> {
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
pub(crate) struct OwnerView<'a> {
    pub(crate) player: usize,
    pub(crate) others: OthersRef<'a>,
    pub(crate) foreign: Foreign<'a>,
    pub(crate) masters: Option<&'a Masters>,
    pub(crate) relations: &'a [u8],
    pub(crate) attributes: &'a [f32],
    pub(crate) civ: u8,
    pub(crate) construction_rate: f32,
    pub(crate) ai_settings: crate::world::player::AiSettings,
}

impl<'a> OwnerView<'a> {
    /// The owner's side: player `new`'s when the unit changed hands to it in its update, else
    /// the updating player's (`c`, its other objects `others`).
    pub(crate) fn of<'c: 'a>(c: &Context<'c>, others: OthersRef<'a>, new: Option<usize>) -> Self {
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
            others: others.changed_hands(new, &q.objects),
            foreign: c.foreign,
            masters: c.every_masters.get(new),
            relations: &q.relations,
            attributes: &q.attributes,
            civ: q.civ,
            construction_rate: q.construction_rate,
            ai_settings: q.ai_settings,
        }
    }
}

/// A unit and the world as its unit AI asks them while it runs, the AI taken out of the unit
/// for that time ([`crate::unit_ai::UnitSide`]): the searches and the unit's own commands.
pub(crate) struct UnitView<'u> {
    /// The unit, without its unit AI.
    pub(crate) unit: &'u mut Unit,
    /// What the attack check of a search reads of the unit.
    pub(crate) attacker: crate::unit_ai::Attacker,
    /// The owner, as the AI runs (a captured unit's new one).
    pub(crate) player: usize,
    /// The owner is a computer player.
    pub(crate) computer: bool,
    /// The world's objects (and the players' scout maps, which an auto scout makes and marks).
    pub(crate) spatial: &'u mut Spatial,
    /// The planners and the mesh, which a reach test walks.
    pub(crate) pathing: &'u mut Pathing,
    /// The world's unit groups, which a reach test's long path reads.
    pub(crate) groups: &'u crate::groups::UnitGroups,
    /// The Object stream, which a reach test's new path draws from.
    pub(crate) object: &'u mut Well512,
    /// The Sprite stream, which a new sprite draws from.
    pub(crate) sprite: &'u mut Well512,
    /// The world's time.
    pub(crate) world_time: u32,
    /// The owner's construction rate.
    pub(crate) construction_rate: f32,
    /// The owner's other objects.
    pub(crate) others: OthersRef<'u>,
    /// The map's terrain zones, which an explorer's search reads.
    pub(crate) zones: &'u mut crate::pathing::zones::Zones,
    /// The other players' lists.
    pub(crate) foreign: Foreign<'u>,
    /// Every player's masters.
    pub(crate) every: &'u [Masters],
    /// The owner's masters.
    pub(crate) masters: Option<&'u Masters>,
    /// The sprites.
    pub(crate) graphics: &'u std::collections::BTreeMap<i16, Graphic>,
    /// The objects an action took (slot 0x178), the caller's.
    pub(crate) takes: &'u mut Vec<(i32, Releaser)>,
    /// The objects an action let go (slot 0x180), the caller's.
    pub(crate) releases: &'u mut Vec<(i32, Releaser)>,
    /// The foundations a counted builder's Build left ([`Context::builder_leaves`]), the
    /// caller's.
    pub(crate) builder_leaves: &'u mut Vec<i32>,
    /// The actions its slots 0x388 and 0x378 gave ([`AiStart`]), the caller's.
    pub(crate) starts: &'u mut Vec<AiStart>,
    /// What its master change (slot 0x90) gives or takes of its owner's attributes, the caller's.
    pub(crate) attribute_adds: &'u mut Vec<(i16, f32)>,
    /// Its master change's tracker promotion ([`Context::promotions`]), the caller's.
    pub(crate) promotions: &'u mut Vec<(i32, i16, i16)>,
    /// What its task search reads of its owner: its diplomacy, attributes and civ, and the drop
    /// sites.
    pub(crate) relations: &'u [u8],
    pub(crate) attributes: &'u [f32],
    pub(crate) civ: u8,
    /// The owner's AI settings.
    pub(crate) ai_settings: crate::world::player::AiSettings,
    pub(crate) drop_sites: &'u [DropSite],
    /// The owner's object tracker, whose done lists the drop-site table's search reads.
    pub(crate) tracker: &'u crate::players::tracker::Tracker,
    /// The game's setting at `+0x680` ([`World::mode_680`]), which a hit's source carries.
    pub(crate) mode_680: i32,
    /// The game's carry rule ([`crate::world::settings::Settings::carry_rule`]), which a hunt's start
    /// reads.
    pub(crate) carry_rule: bool,
}

#[cfg(test)]
mod tests;
