//! The views a unit's update and its unit AI read the world through ([`UnitView`],
//! [`Context`], [`Foreign`], [`OwnerView`], the pickers).

use crate::build::Walk;
use crate::damage::Victim;
use crate::masters::Masters;
use crate::movement::{Me, Mover, Planning, footprint_gap_squared};
use crate::pathing::Pathing;
use crate::rng::{Stream, Streams, Well512};
use crate::spatial::{Placed, Spatial};
use crate::sprite::Graphic;
use crate::tasks::{Actor, DropSite};
use crate::turn::player::Player;
use crate::turn::unit::{AiResume, Unit};
use crate::turn::world::{
    Releaser, ResourceWorld, SideWorld, closest_resource_of, herded, task_search, task_target_with,
    victim_in, victim_of,
};
use crate::turn::{BuildingDeath, PETARD, Recycling};
use crate::unit_ai::{AiMove, GenieError, Search, UnitAi};

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
    pub zones: &'a mut crate::zones::Zones,
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
}
pub(in crate::turn) struct Context<'a> {
    /// The tick's dt in milliseconds, and in seconds (`world+0x164`).
    pub(in crate::turn) dt: u32,
    pub(in crate::turn) dt_seconds: f32,
    /// The ticks so far, this one included.
    pub(in crate::turn) tick: u32,
    /// The owner's accumulated time when it has the turn: the full unit-AI update runs.
    pub(in crate::turn) turn: Option<u32>,
    pub(in crate::turn) world_time: u32,
    pub(in crate::turn) computer: bool,
    pub(in crate::turn) streams: &'a mut Streams,
    /// Every object's place, which a walker reads and its moves update.
    pub(in crate::turn) spatial: &'a mut Spatial,
    /// The planners and the mesh, which a walker's path request uses.
    pub(in crate::turn) pathing: &'a mut Pathing,
    /// The world's unit groups, which a walker's long path reads.
    pub(in crate::turn) groups: &'a crate::group::UnitGroups,
    /// The owner's construction rate (attribute 195).
    pub(in crate::turn) construction_rate: f32,
    /// The owner's masters, which an attack reads.
    pub(in crate::turn) masters: Option<&'a Masters>,
    /// The target of the unit's attack, read before its update and written back after it, and
    /// the objects about a unit with an area effect, which its blast may reach.
    pub(in crate::turn) victim: Option<&'a mut Victim>,
    /// The unit AI's current target as its target block reads it ([`Turns::seen`]).
    pub(in crate::turn) ai_target: Result<Option<crate::unit_ai::Seen>, GenieError>,
    pub(in crate::turn) blasted: Vec<Victim>,
    /// The unit was killed in its update (a retaliation), which the caller handles after it.
    pub(in crate::turn) own_killed: Option<crate::damage::Kill>,
    /// What the unit's death leaves for the caller: the remains its dying sprite's end makes
    /// (`FUN_1411a4e90`: the master, the place and the dead object's facet) and the paid cost its
    /// owner takes back (attributes 98 and 164).
    pub(in crate::turn) remains: Option<(i16, [f32; 3], Option<u8>)>,
    pub(in crate::turn) refund: Vec<(i16, f32)>,
    /// What a built building's death takes from its owner before its combat part's refund: the
    /// counts of the monastery, the wonder and the castle (slot 0xf8, then slot 0xf0 with 0 for
    /// a value below 0), and its master's technology (`+0x98c`) the caller withdraws when no
    /// other building of the master stands ([`Turns::withdraw_tech`]).
    pub(in crate::turn) building_death: Option<BuildingDeath>,
    /// The pauses the unit's walk gave units it met ([`Mover::pauses`]).
    pub(in crate::turn) pauses: Vec<(i32, f32)>,
    /// The gate's update asked its player to open or shut it ([`Turns::toggle_gate`], the
    /// caller's after the update).
    pub(in crate::turn) gate_toggle: bool,
    /// The unit's update stopped before its unit AI for the caller's work order
    /// ([`Unit::ai_part`]); given back, the update runs only that part.
    pub(in crate::turn) ai_resume: Option<AiResume>,
    /// The player the unit's own AI gave it to (slot 0x70, [`Unit::ai_change_owner`]) and
    /// whether it has the unit's master: the update ends with the takeover ([`Step::Captured`]).
    pub(in crate::turn) taken_over: Option<(usize, bool)>,
    /// The unit is a building whose slot 0x70 ran in the update: the whole change is the
    /// caller's ([`crate::turn::unit::Captured::building`]).
    pub(in crate::turn) building_handover: bool,
    /// The objects the unit's actions let go (slot 0x180, [`Turns::release`]) and take (slot
    /// 0x178, [`Turns::take`]) in other players' lists, which the caller applies after the hit's
    /// write-backs.
    pub(in crate::turn) releases: Vec<(i32, Releaser)>,
    pub(in crate::turn) takes: Vec<(i32, Releaser)>,
    /// The foundations a counted builder's Build leaves as the unit's action list goes
    /// ([`Unit::clear_own_actions`], `FUN_140aadbf0`): the building `FUN_140aaddb0` gives counts one
    /// builder less ([`Turns::leave_builders`]), the caller's before the releases.
    pub(in crate::turn) builder_leaves: Vec<i32>,
    /// The unit group a unit that entered a building left (`FUN_1411921d0`), with the
    /// building: the caller runs the group's removeUnit ([`Turns::remove_unit`]).
    pub(in crate::turn) left_group: Option<(i32, i32)>,
    /// The objects the unit made (a ranged unit's missiles), which join the end of its owner's
    /// list after it and update later in the same tick.
    pub(in crate::turn) made: Vec<Unit>,
    /// The objects whose master leaves ghosts that died in this update ([`crate::ghost`]), the
    /// caller's.
    pub(in crate::turn) ghost_deaths: Vec<i32>,
    /// The buildings whose construction started in this update: their owner watches them
    /// ([`Turns::watch_started`]), the caller's.
    pub(in crate::turn) started: Vec<i32>,
    /// The buildings that died in this update: their owner's watched list lets them go
    /// (`FUN_141091310`), the caller's.
    pub(in crate::turn) unwatched: Vec<i32>,
    /// The annexes a building's `set_state` (`FUN_1411dbb70`) took to its own state (slot 0xa0
    /// on each, before its own parts), the caller's after the update ([`Turns::set_state`]).
    pub(in crate::turn) annex_states: Vec<(i32, u8)>,
    /// The annexes a dying building let go (the static update's death, `0x14118ad50`), each with
    /// the hit points it takes: its `+0xbb8` clears, the caller's after the update.
    pub(in crate::turn) released_annexes: Vec<(i32, f32)>,
    /// The objects a builder's footprint clearing deletes (the Build's `set_state(7)`,
    /// `FUN_1415c3620` with `param_6`; their records went already), the caller's after the
    /// update ([`Turns::clear_object`]).
    pub(in crate::turn) cleared: Vec<crate::build::Cleared>,
    /// The tracker's promotions (`FUN_14104fb20`) of the owner's units whose master changed
    /// while the unit AI ran (the unit, its old master, its new one), the caller's after the
    /// update ([`Turns::track_promote`]).
    pub(in crate::turn) promotions: Vec<(i32, i16, i16)>,
    /// The flares a hit on a player's object has that player make (master 112, its slot 0x1c0)
    /// at the attacker's place, doubled in their amount for a trebuchet's hit, the caller's
    /// after the update ([`Turns::make_flare`]).
    pub(in crate::turn) flares: Vec<(usize, [f32; 3], bool)>,
    /// The foundations another player's marks lie on, which a builder's `set_state(7)` removes
    /// when their owner is human ([`crate::build::BuildStep::removes`]), the caller's after the
    /// update ([`Turns::remove_building`]).
    pub(in crate::turn) site_removals: Vec<i32>,
    /// The units on a foundation's footprint its builder's test met (the foundation, them):
    /// the caller sends off those the test would ([`Turns::evacuate`]).
    pub(in crate::turn) evacuations: Vec<(i32, Vec<i32>)>,
    /// The world's reusable ids and pools.
    pub(in crate::turn) recycling: &'a mut Recycling,
    /// The game's setting at `+0x680` is 1 ([`Turns::mode_680`]).
    pub(in crate::turn) mode_1: bool,
    /// The owner's attributes, which a gatherer's productivity reads.
    pub(in crate::turn) attributes: &'a [f32],
    /// The owner's food, wood, stone and gold ([`Player::resources`]), which a repair pays from.
    pub(in crate::turn) resources: [f32; 4],
    /// The graphics, for the sprites a hunter takes.
    pub(in crate::turn) graphics: &'a std::collections::BTreeMap<i16, Graphic>,
    /// The drop-site table, whose livestock entries a hunt leads its livestock to.
    pub(in crate::turn) drop_sites: &'a [DropSite],
    /// The owner's id and civ (`player+0x738`).
    pub(in crate::turn) player: usize,
    pub(in crate::turn) civ: u8,
    /// The owner's AI settings (`+0x608`..), which its unit AIs read.
    pub(in crate::turn) ai_settings: crate::turn::player::AiSettings,
    /// The player shares its gathered resources with its allies (`+0x1426`).
    pub(in crate::turn) sharing: bool,
    /// The game's farm setting ([`crate::settings::Settings::walkable_farms`]).
    pub(in crate::turn) farm_stops: bool,
    /// The game's setting [`crate::settings::Settings::predator_hunts_end`].
    pub(in crate::turn) predator_hunts_end: bool,
    /// Every player's masters and attribute 97 (by player), for a capture.
    pub(in crate::turn) every_masters: &'a [Masters],
    pub(in crate::turn) attribute_97: &'a [f32],
    /// The player whose turn this tick is (`world+0x188`), every player's accumulated time
    /// (`world+0x18c`, as it stands when the object updates) and computer flag.
    pub(in crate::turn) player_turn: usize,
    pub(in crate::turn) player_time: &'a [u32],
    pub(in crate::turn) computers: &'a [bool],
    /// Each player's `chance_to_dodge_missiles` ([`crate::turn::Player::dodge_chance`]).
    pub(in crate::turn) dodge_chances: &'a [u8],
    /// Each player's `chance_for_archers_to_maintain_distance`
    /// ([`crate::turn::Player::keep_distance_chance`]).
    pub(in crate::turn) keep_distance_chances: &'a [u8],
    /// Some player's attribute 204 is not 0, which a building's completion reads.
    pub(in crate::turn) attribute_204: bool,
    /// [`Turns::mode_680`].
    pub(in crate::turn) mode_680: i32,
    /// The game mode is one of `FUN_141302540`'s ([`scenario_mode`]).
    pub(in crate::turn) scenario: bool,
    /// The game mode's descriptor flag `+0x9c` ([`standard_mode`]) and each player's computer
    /// flag and `mAllowCapturingGaiaUnits`, which a takeover by sight reads
    /// ([`crate::visibility::sight_taker`]).
    pub(in crate::turn) standard: bool,
    pub(in crate::turn) sight_takers: &'a [(bool, bool)],
    /// The owner's object tracker, which a completion's first part (`FUN_1411c6bd0`) moves a
    /// building in, and the unit lines its keys read ([`crate::tracker`]).
    pub(in crate::turn) tracker: &'a mut crate::tracker::Tracker,
    pub(in crate::turn) lines: &'a crate::lines::UnitLines,
    /// The attributes the owner gains from the object's update (a building it completed), which
    /// the caller adds after it.
    pub(in crate::turn) attribute_adds: Vec<(i16, f32)>,
    /// The events the owner's history records from the object's update (a castle or a wonder it
    /// completed, [`crate::build::Built::history`]), which the caller adds after it.
    pub(in crate::turn) history_events: Vec<u8>,
    /// The technologies the owner researches from the object's update (a completed building's),
    /// which the caller completes after the attributes ([`Turns::complete_research`]).
    pub(in crate::turn) techs_done: Vec<i16>,
    /// The mule carts the object's update completed, whose search for resources the caller runs
    /// after it ([`Turns::cart_search`]).
    pub(in crate::turn) carts: Vec<(i32, u8)>,
    /// The buildings the object's update completed with a stack unit or that disappear when
    /// built: the caller makes the stack unit ([`Turns::stack_unit`]) before the technologies
    /// and destroys the building (slot 0xd0) after them.
    pub(in crate::turn) completions: Vec<(i32, Option<crate::build::StackUnit>, bool)>,
    /// The work a builder found after its building (slot 0x7e8 from the Build's case 0xdf),
    /// which the caller gives after the update ([`Turns::order_work`]).
    pub(in crate::turn) work_order: Option<crate::order::Work>,
    /// The Transport's unload in its state 6 (`FUN_140aa3ba0`), which the caller runs after the
    /// actions and before the unit AI part ([`Turns::transport_unload`]).
    pub(in crate::turn) transport_unload: bool,
    /// The unit AI's order on a target (`0x2db`) the unit took from its queue, which the caller
    /// runs after the update ([`Turns::order_on_target`]), and the full update's tail that reads
    /// what the order gave ([`crate::unit_ai::UnitAi::after_order`]).
    pub(in crate::turn) order_on_target: Option<(
        crate::unit_ai::QueuedOrder,
        Option<crate::unit_ai::AfterOrder>,
    )>,
    /// The unit AI's `HuntObject`s, `GatherObject`s and works (slot 0x7e8) in the order it gave
    /// them, whose unit parts the caller runs after the update in that order ([`AiWork`]): a
    /// later one's stop tears down the action an earlier one made.
    pub(in crate::turn) works: Vec<AiWork>,
    /// The foundation of the unit AI's `BuildObject`, which the caller builds after the update
    /// ([`Turns::build_object`]).

    /// The foundation of the unit's own build command (slot 0x380, a Gather's `set_state(6)` on
    /// a target not built yet), which the caller runs after the update ([`Turns::build_command`]).
    pub(in crate::turn) build_command: Option<i32>,
    /// A Gather's walk back to reseed ended (state 0xf2, whether it arrived), which the caller
    /// reseeds after the update ([`Turns::reseed_after_walk`]).
    pub(in crate::turn) reseed: Option<bool>,
    /// The unit AI's moves off foundations' footprints (slot 0x148), in order, which the caller
    /// runs after the update ([`Turns::move_aside`]).
    pub(in crate::turn) move_aside: Vec<AiMove>,
    /// The transports an Enter's `set_state(4)` sent to where they meet the unit (slot 0x340),
    /// which the caller moves after the update ([`Turns::move_transport`]'s move).
    pub(in crate::turn) transport_moves: Vec<(i32, [f32; 3])>,
    /// Another player's transport or building the unit entered, with what its volleys gain: the
    /// caller adds them and the unit to its list after the update.
    pub(in crate::turn) entered: Option<(i32, f32)>,
    /// The actions the unit's slots 0x388 and 0x378 gave while its unit AI ran ([`AiStart`]),
    /// which the caller makes the unit's after the update, in order.
    pub(in crate::turn) starts: Vec<AiStart>,
    /// The work a builder gave an annex whose main building takes it (the foundation, the
    /// seconds; [`crate::build::Worked::Main`]), which the caller runs after the update
    /// ([`Turns::work_building`]).
    pub(in crate::turn) main_works: Vec<(i32, f32)>,
    /// What a repair changed of another player's objects ([`RepairWrite`]), which the caller
    /// writes after the update.
    pub(in crate::turn) repair_writes: Vec<RepairWrite>,
    /// The owner's diplomacy (`player+0x260`), which the unit's task search reads.
    pub(in crate::turn) relations: &'a [u8],
    /// The livestock a hunt sends to a drop site with its herd, which the caller sends after the
    /// update ([`Turns::lead_herd`]).
    pub(in crate::turn) leads: Option<(i32, i32)>,
    /// The map's zone maps, which an auto scout's search reads.
    pub(in crate::turn) zones: &'a mut crate::zones::Zones,
    /// The other players' lists, as they stand before the unit's update.
    pub(in crate::turn) foreign: Foreign<'a>,
}

/// What an object's step gathered in its Context for the world to apply after it
/// ([`crate::turn::Turns::finish_object`]): the Context's fields of the same meaning; the left
/// group carries the unit's id.
pub(in crate::turn) struct ObjectOutputs {
    /// [`Context`]'s `attribute_adds`.
    pub(in crate::turn) adds: Vec<(i16, f32)>,
    /// [`Context`]'s `order_on_target`.
    pub(in crate::turn) ordered: Option<(
        crate::unit_ai::QueuedOrder,
        Option<crate::unit_ai::AfterOrder>,
    )>,
    /// [`Context`]'s `history_events`.
    pub(in crate::turn) history_events: Vec<u8>,
    /// [`Context`]'s `works`.
    pub(in crate::turn) works: Vec<AiWork>,
    /// [`Context`]'s `leads`.
    pub(in crate::turn) leads: Option<(i32, i32)>,
    /// [`Context`]'s `build_command`.
    pub(in crate::turn) build_command: Option<i32>,
    /// [`Context`]'s `reseed`.
    pub(in crate::turn) reseed: Option<bool>,
    /// [`Context`]'s `move_aside`.
    pub(in crate::turn) move_aside: Vec<AiMove>,
    /// [`Context`]'s `transport_moves`.
    pub(in crate::turn) transport_moves: Vec<(i32, [f32; 3])>,
    /// [`Context`]'s `entered`.
    pub(in crate::turn) entered: Option<(i32, f32)>,
    /// [`Context`]'s `starts`.
    pub(in crate::turn) starts: Vec<AiStart>,
    /// [`Context`]'s `techs_done`.
    pub(in crate::turn) techs_done: Vec<i16>,
    /// [`Context`]'s `carts`.
    pub(in crate::turn) carts: Vec<(i32, u8)>,
    /// [`Context`]'s `completions`.
    pub(in crate::turn) completions: Vec<(i32, Option<crate::build::StackUnit>, bool)>,
    /// [`Context`]'s `work_order`.
    pub(in crate::turn) work_order: Option<crate::order::Work>,
    /// [`Context`]'s `transport_unload`.
    pub(in crate::turn) transport_unload: bool,
    /// [`Context`]'s `blasted`.
    pub(in crate::turn) blasted: Vec<Victim>,
    /// [`Context`]'s `own_killed`.
    pub(in crate::turn) own_killed: Option<crate::damage::Kill>,
    /// [`Context`]'s `remains`.
    pub(in crate::turn) remains: Option<(i16, [f32; 3], Option<u8>)>,
    /// [`Context`]'s `refund`.
    pub(in crate::turn) refund: Vec<(i16, f32)>,
    /// [`Context`]'s `annex_states`.
    pub(in crate::turn) annex_states: Vec<(i32, u8)>,
    /// [`Context`]'s `cleared`.
    pub(in crate::turn) cleared: Vec<crate::build::Cleared>,
    /// [`Context`]'s `site_removals`.
    pub(in crate::turn) site_removals: Vec<i32>,
    /// [`Context`]'s `flares`.
    pub(in crate::turn) flares: Vec<(usize, [f32; 3], bool)>,
    /// [`Context`]'s `promotions`.
    pub(in crate::turn) promotions: Vec<(i32, i16, i16)>,
    /// [`Context`]'s `released_annexes`.
    pub(in crate::turn) released_annexes: Vec<(i32, f32)>,
    /// [`Context`]'s `main_works`.
    pub(in crate::turn) main_works: Vec<(i32, f32)>,
    /// [`Context`]'s `building_death`.
    pub(in crate::turn) building_death: Option<BuildingDeath>,
    /// [`Context`]'s `pauses`.
    pub(in crate::turn) pauses: Vec<(i32, f32)>,
    /// [`Context`]'s `gate_toggle`.
    pub(in crate::turn) gate_toggle: bool,
    /// [`Context`]'s `ai_resume`.
    pub(in crate::turn) ai_resume: Option<AiResume>,
    /// [`Context`]'s `releases`.
    pub(in crate::turn) releases: Vec<(i32, Releaser)>,
    /// [`Context`]'s `builder_leaves`.
    pub(in crate::turn) builder_leaves: Vec<i32>,
    /// [`Context`]'s `left_group`.
    pub(in crate::turn) left_group: Option<(i32, i32, i32)>,
    /// [`Context`]'s `takes`.
    pub(in crate::turn) takes: Vec<(i32, Releaser)>,
    /// [`Context`]'s `made`.
    pub(in crate::turn) made: Vec<Unit>,
    /// [`Context`]'s `ghost_deaths`.
    pub(in crate::turn) ghost_deaths: Vec<i32>,
    /// [`Context`]'s `started`.
    pub(in crate::turn) started: Vec<i32>,
    /// [`Context`]'s `unwatched`.
    pub(in crate::turn) unwatched: Vec<i32>,
    /// [`Context`]'s `evacuations`.
    pub(in crate::turn) evacuations: Vec<(i32, Vec<i32>)>,
    /// [`Context`]'s `repair_writes`.
    pub(in crate::turn) repair_writes: Vec<RepairWrite>,
}

impl Context<'_> {
    /// What the step gathered ([`ObjectOutputs`]), taken out of the Context; `id` is the
    /// unit's.
    pub(in crate::turn) fn outputs(&mut self, id: i32) -> ObjectOutputs {
        ObjectOutputs {
            adds: std::mem::take(&mut self.attribute_adds),
            ordered: self.order_on_target.take(),
            history_events: std::mem::take(&mut self.history_events),
            works: std::mem::take(&mut self.works),
            leads: self.leads.take(),
            build_command: self.build_command.take(),
            reseed: self.reseed.take(),
            move_aside: std::mem::take(&mut self.move_aside),
            transport_moves: std::mem::take(&mut self.transport_moves),
            entered: self.entered.take(),
            starts: std::mem::take(&mut self.starts),
            techs_done: std::mem::take(&mut self.techs_done),
            carts: std::mem::take(&mut self.carts),
            completions: std::mem::take(&mut self.completions),
            work_order: self.work_order.take(),
            transport_unload: std::mem::take(&mut self.transport_unload),
            blasted: std::mem::take(&mut self.blasted),
            own_killed: self.own_killed.take(),
            remains: self.remains.take(),
            refund: std::mem::take(&mut self.refund),
            annex_states: std::mem::take(&mut self.annex_states),
            cleared: std::mem::take(&mut self.cleared),
            site_removals: std::mem::take(&mut self.site_removals),
            flares: std::mem::take(&mut self.flares),
            promotions: std::mem::take(&mut self.promotions),
            released_annexes: std::mem::take(&mut self.released_annexes),
            main_works: std::mem::take(&mut self.main_works),
            building_death: self.building_death.take(),
            pauses: std::mem::take(&mut self.pauses),
            gate_toggle: std::mem::take(&mut self.gate_toggle),
            ai_resume: self.ai_resume.take(),
            releases: std::mem::take(&mut self.releases),
            builder_leaves: std::mem::take(&mut self.builder_leaves),
            left_group: self.left_group.take().map(|(g, b)| (g, id, b)),
            takes: std::mem::take(&mut self.takes),
            made: std::mem::take(&mut self.made),
            ghost_deaths: std::mem::take(&mut self.ghost_deaths),
            started: std::mem::take(&mut self.started),
            unwatched: std::mem::take(&mut self.unwatched),
            evacuations: std::mem::take(&mut self.evacuations),
            repair_writes: std::mem::take(&mut self.repair_writes),
        }
    }
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
    pub(in crate::turn) zones: &'u mut crate::zones::Zones,
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
    /// The game's setting at `+0x680` ([`Turns::mode_680`]), which a hit's source carries.
    pub(in crate::turn) mode_680: i32,
}

impl UnitView<'_> {
    /// The world as `bestUnitToAttack` and the attack tests read it for the unit
    /// ([`WorldPicker`]), with `seeker` and without a terrain restriction.
    const fn picker(&self, seeker: Option<crate::unit_ai::Seeker>) -> WorldPicker<'_> {
        WorldPicker {
            attacker: &self.attacker,
            seeker,
            me: self.unit.id,
            player: self.player,
            others: self.others,
            foreign: self.foreign,
            spatial: self.spatial,
            every: self.every,
            restriction: -1,
            unit: &*self.unit,
            attributes: self.attributes,
            relations: self.relations,
            ai_settings: self.ai_settings,
            now: self.world_time,
            mode_680: self.mode_680,
        }
    }

    /// `AskForHelp`'s tests of a helper `u` (of player `owner`, master `m`) that passed its
    /// class, owner and reach tests, against `attacker`, all reads (the game asks its attack
    /// check before its owner's test; neither changes what the other reads): a helper in no group
    /// helps only when idle (no action value, no action, no order, an empty queue and no target:
    /// "He has a `UnitAIaction`"), one in a group only when the group patrols (`FUN_141017c30` or
    /// `FUN_141017c40`); then its top action's target within its range (slot 400, slot 0x1e8:
    /// the footprints' squared gap between the least and the weapon range squared) keeps it, but
    /// a building whose master has no attack for a patrolling group's member; then its unit AI's
    /// attack check (slot 0xa0 with 0, [`crate::unit_ai::can_attack`]) on the attacker. True for
    /// a helper that answers.
    pub(in crate::turn) fn helper_free(
        &self,
        owner: usize,
        u: &Unit,
        m: &crate::masters::Master,
        attacker: i32,
    ) -> Result<bool, GenieError> {
        let (others, foreign, player) = (self.others, self.foreign, self.player);
        let find = |id: i32| {
            others
                .iter()
                .find(|o| o.id == id)
                .map(|o| (player, o))
                .or_else(|| foreign.find(id))
        };
        let master_of =
            |q: usize, o: &Unit| self.every.get(q).and_then(|ms| ms.get(o.life.master_id));
        let Some(ai) = u.ai.as_ref() else {
            return Ok(false);
        };
        let patrolling = match self.groups.get(u.group.id) {
            None => {
                let busy = u.top_action_kind().is_some() || u.unmodelled_action.is_some();
                if ai.current_action != -1
                    || busy
                    || ai.current_order != -1
                    || !ai.orders.is_empty()
                    || ai.current_target != -1
                {
                    return Ok(false);
                }
                false
            }
            Some(g) if g.patrols() => true,
            Some(_) => return Ok(false),
        };
        if let Some((tq, t)) = u.top_target().filter(|&t| t != -1).and_then(find) {
            let tm = master_of(tq, t).ok_or(NOT_HELD)?;
            let gap = footprint_gap_squared(u.position, m.radius, t.position, tm.radius);
            let in_range =
                gap <= m.weapon_range * m.weapon_range && m.min_range * m.min_range <= gap;
            let spared = patrolling && tm.kind == crate::order::BUILDING && tm.weapons.is_empty();
            if in_range && !spared {
                return Ok(false);
            }
        }
        let computer = if owner == self.player {
            self.computer
        } else {
            self.foreign.player(owner).is_some_and(|q| q.computer)
        };
        let capture = self
            .spatial
            .sight
            .as_ref()
            .and_then(|v| v.seer(u.id))
            .map_or(0, |s| s.capture);
        let me = u.attacker_of(
            ai.ai_type,
            Some(m),
            (owner, computer),
            self.attacker.scenario,
            capture,
        );
        let target_kind = (ai.current_target != -1)
            .then(|| {
                find(ai.current_target)
                    .and_then(|(q, o)| master_of(q, o))
                    .map(|tm| tm.kind)
            })
            .flatten();
        let picker = self.picker(None);
        let (target, _) = picker.target(attacker)?;
        crate::unit_ai::can_attack(&me, &ai.seeker_with(target_kind), &target)
    }

    /// The forced attack command on the target of the unit's own Attack (`FUN_141081a80`): the
    /// top Attack's slot 0x48 (`FUN_140a841d0`, [`crate::attack::Attack::work`]) on the target
    /// and its place, with the main building of an annex for a unit of an area effect, the
    /// unit's top speed and canMoveToAttack ([`crate::unit_ai::Seeker::can_move`]); what it takes
    /// and lets go is the caller's ([`UnitView::takes`], [`UnitView::releases`]), a new target
    /// clearing the unit's attack count (`+0x900`). Its answer is not read.
    fn attack_again(
        &mut self,
        target: i32,
        t: &Placed,
        master: &crate::masters::Master,
        ai: &UnitAi,
    ) -> Result<(), GenieError> {
        let mark = |id: i32, p: &Placed| -> Result<crate::attack::Mark, GenieError> {
            let (_, u) = self
                .foreign
                .find(id)
                .or_else(|| self.others.iter().find(|u| u.id == id).map(|u| (0, u)))
                .ok_or(NOT_HELD)?;
            let m = self
                .every
                .get(p.player)
                .and_then(|ms| ms.get(p.master))
                .ok_or(NOT_HELD)?;
            Ok(crate::attack::Mark {
                id,
                kind: m.kind,
                class: m.class,
                state: u.life.state,
                position: p.position,
                radius: p.radius,
            })
        };
        let target_mark = mark(target, t)?;
        let main = match self
            .foreign
            .find(target)
            .map(|(_, u)| u.annex_of)
            .or_else(|| {
                self.others
                    .iter()
                    .find(|u| u.id == target)
                    .map(|u| u.annex_of)
            })
            .flatten()
        {
            Some(m) => match self.spatial.get(m).copied() {
                Some(p) => Some(mark(m, &p)?),
                None => None,
            },
            None => None,
        };
        let unit = &mut *self.unit;
        let wielder = crate::attack::Wielder {
            area_effect: master.area_effect,
            speed: unit.mover.as_ref().map_or(0.0, |m| m.stats.speed),
            position: unit.position,
            radius: master.radius,
        };
        let can_move = ai.current_order != -1 || ai.stance <= 1;
        let attack = unit.attack.as_mut().expect("the caller's own Attack");
        let w = attack.work(Some(target_mark), main, &wielder, |_| can_move);
        let by = Releaser {
            id: unit.id,
            class: master.class,
            player: self.player,
        };
        if w.clears_count {
            unit.attack_count = 0;
        }
        if let Some(old) = w.lets_go {
            self.releases.push((old, by));
        }
        if let Some(new) = w.takes {
            self.takes.push((new, by));
        }
        Ok(())
    }

    /// What a hit of the unit reads of the object `id` ([`victim_in`]): one of the owner's live
    /// objects or another player's; None for an object the view does not hold (the owner's
    /// sleeping objects, or the old owner's of a unit taken over in its update, whose attributes
    /// it does not hold).
    fn victim_of_target(&self, id: i32) -> Option<Victim> {
        let (q, t, attributes) = if let Some(t) = self.others.iter().find(|u| u.id == id) {
            (self.player, t, self.attributes)
        } else {
            let (q, t) = self.foreign.find(id)?;
            (q, t, &self.foreign.player(q)?.attributes[..])
        };
        let master = self.every.get(q)?.get(t.life.master_id)?;
        let dip = SideWorld {
            player: self.player,
            relations: self.relations,
            foreign: self.foreign,
            drop_sites: self.drop_sites,
        };
        Some(victim_in(
            t,
            (q, master, attributes),
            (self.player, &dip),
            self.spatial,
            self.mode_680,
        ))
    }

    /// The start's `set_state(6)` of the unit's new Attack ([`crate::attack::Attack::enter_ready`])
    /// with what its readyToAttack and its swing read: the target as a hit reads it
    /// ([`UnitView::victim_of_target`]), the unit as a combat object, its unit AI's stance, leash
    /// and queue. Returns the notifies it posted. Its swing lands no hit and fires nothing before
    /// its update, nor does readyToAttack let the target go or send the unit home there.
    fn ready_at_start(
        &mut self,
        attack: &mut crate::attack::Attack,
        master: &crate::masters::Master,
        ai: &UnitAi,
    ) -> Result<Vec<(i32, [i32; 3])>, GenieError> {
        let mut victim = self
            .victim_of_target(attack.target)
            .ok_or(GenieError::unsupported(
                "an attack's target the slice does not hold live",
            ))?;
        let unit = &mut *self.unit;
        let own = victim_of(
            unit,
            master,
            self.attributes,
            self.player,
            self.spatial.terrain.as_ref(),
            self.mode_680,
        );
        let me = Me {
            id: unit.id,
            position: unit.position,
            group: unit.group.id,
            action_state: crate::attack::READY,
        };
        let walk = Walk {
            mover: unit.mover.as_mut(),
            me,
            dt: 0.0,
            world: self.spatial,
            plan: None,
        };
        let mut blasted = Vec::new();
        let mut strike = crate::attack::Strike {
            sprite: &mut unit.sprite,
            anim_speed: &mut unit.anim_speed,
            standing: self.graphics.get(&master.standing_sprite).copied(),
            volleys: self
                .masters
                .is_some_and(|m| crate::attack::volleys(master, m)),
            attack_timer: &mut unit.attack_timer,
            reload_worldtime: &mut unit.reload_worldtime,
            reload: unit.reload_stinger + (master.reload - unit.reload_powerup),
            hit_chance: master.hit_chance,
            victim: &mut victim,
            blasted: &mut blasted,
            sprite_rng: self.sprite,
            own,
            executed: None,
            fire: None,
            can_move: ai.current_order != -1 || ai.stance <= 1,
            stance: ai.stance,
            state_position: ai.state_position,
            posted: Vec::new(),
            queue_empty: ai.orders.is_empty(),
            world_time: self.world_time,
            ai_order: Some(ai.current_order),
            ai_type: Some(ai.ai_type),
            computer: self.computer,
            armed: crate::turn::unit::armed(master, unit.firepower),
            volley_under_way: unit.next_volley != 0,
            let_go: None,
            forgets: false,
            sends_home: false,
            swing_sprites: crate::attack::swing_sprites(
                unit.abilities.look.as_ref(),
                master,
                self.graphics,
            ),
        };
        attack.enter_ready(walk, &mut strike)?;
        // set_state 6 and 7 hit nothing, fire nothing and let nothing go: the update does
        debug_assert!(strike.executed.is_none() && strike.fire.is_none());
        debug_assert!(strike.let_go.is_none() && !strike.forgets && !strike.sends_home);
        Ok(std::mem::take(&mut strike.posted))
    }
}

/// Slot 0x228 (`FUN_141084070`): the sum of master `m`'s attacks above 0 (`COMISS`), from +0.
fn attacks(m: &crate::masters::Master) -> f32 {
    m.weapons
        .iter()
        .filter(|w| 0 < w.1)
        .fold(0.0, |sum, w| sum + f32::from(w.1))
}

/// The hit points (`DAT_14330ebc8`) and the range (`DAT_142e7f384`) below which a villager
/// fights its attacker back ([`WorldPicker::harmless`]).
const HARMLESS_HP: f32 = crate::image::unit_ai::HARMLESS_HP;
const HARMLESS_RANGE: f32 = crate::image::unit_ai::HARMLESS_RANGE;

/// An object the unit AI found or names that the slice does not hold (another player's it does
/// not keep).
const NOT_HELD: GenieError = GenieError::unsupported("a found object the slice does not hold");

/// What `bestUnitToAttack` reads of the world for the unit whose AI runs
/// ([`crate::unit_ai::AttackPicker`]): the objects it found, by what the world holds of them, and
/// the unit's quick path to them.
pub(in crate::turn) struct WorldPicker<'a> {
    /// What the tests read of the unit.
    pub(in crate::turn) attacker: &'a crate::unit_ai::Attacker,
    /// What they read of its unit AI; None outside `bestUnitToAttack`.
    pub(in crate::turn) seeker: Option<crate::unit_ai::Seeker>,
    /// The unit.
    pub(in crate::turn) me: i32,
    /// Its owner.
    pub(in crate::turn) player: usize,
    /// The owner's other objects.
    pub(in crate::turn) others: &'a [Unit],
    /// The other players' lists.
    pub(in crate::turn) foreign: Foreign<'a>,
    /// The world's objects.
    pub(in crate::turn) spatial: &'a Spatial,
    /// Every player's masters.
    pub(in crate::turn) every: &'a [Masters],
    /// The unit's master's terrain restriction (`+0x606`).
    pub(in crate::turn) restriction: i16,
    /// The unit itself.
    pub(in crate::turn) unit: &'a Unit,
    /// Its owner's attributes, diplomacy and AI settings.
    pub(in crate::turn) attributes: &'a [f32],
    /// Its owner's diplomacy (`player+0x260`).
    pub(in crate::turn) relations: &'a [u8],
    /// Its owner's AI settings.
    pub(in crate::turn) ai_settings: crate::turn::player::AiSettings,
    /// The world's time.
    pub(in crate::turn) now: u32,
    /// The game's setting `+0x680`.
    pub(in crate::turn) mode_680: i32,
}

impl WorldPicker<'_> {
    /// The object `id` (another player's, the owner's others or the unit itself) with its owner,
    /// its owner's attributes and its master (its own, else its owner's).
    fn held(&self, id: i32) -> Option<(usize, &Unit, &[f32], &crate::masters::Master)> {
        let (q, u, attributes) = if id == self.me {
            (self.player, self.unit, self.attributes)
        } else if let Some(u) = self.others.iter().find(|u| u.id == id) {
            (self.player, u, self.attributes)
        } else {
            let (q, u) = self.foreign.find(id)?;
            (q, u, &self.foreign.player(q)?.attributes[..])
        };
        let master = match u.own_master.as_deref() {
            Some(m) => m,
            None => self.every.get(q)?.get(u.life.master_id)?,
        };
        Some((q, u, attributes, master))
    }

    /// The priest's slot 0x1b8 of object `who` (a unit AI of another type answers false): may it
    /// convert object `whom`, `any` not asking its faith ([`crate::unit_ai::can_convert`]).
    fn converts(&self, who: i32, whom: i32, any: bool) -> Result<bool, GenieError> {
        let (_, p, attributes, pm) = self.held(who).ok_or(GenieError::unsupported(
            "a priest's conversion test of an object the slice does not hold",
        ))?;
        if p.ai
            .as_ref()
            .is_none_or(|a| a.ai_type != crate::unit_ai::PRIEST_AI)
        {
            return Ok(false);
        }
        let converter = crate::unit_ai::Converter {
            class: pm.class,
            master: p.life.master_id,
            faith: p.life.amount,
            attributes,
            tasks: &pm.tasks,
        };
        let target = self
            .held(whom)
            .map(|(q, t, _, tm)| crate::unit_ai::Convertee {
                owner: q,
                class: tm.class,
                master: t.life.master_id,
                hero: tm.hero_flag,
            });
        Ok(crate::unit_ai::can_convert(
            &converter,
            target.as_ref(),
            any,
        ))
    }

    /// What object `from` deals object `to` per hit (`to`'s slot 0x220 with `from`,
    /// `FUN_141083f20`): `from`'s elevation factor against it (slot 0x1f0: a combat object's
    /// [`crate::damage::elevation_factor`], a building's [`crate::damage::building_elevation_factor`]
    /// over it), then `to`'s armor (slot 0xf0, [`crate::damage::armor_damage`]) against `from`'s
    /// attacks.
    fn damage(&self, from: i32, to: i32) -> Result<f32, GenieError> {
        let (fq, f, fa, fm) = self.held(from).ok_or(NOT_HELD)?;
        let (tq, t, ta, tm) = self.held(to).ok_or(NOT_HELD)?;
        let terrain = self.spatial.terrain.as_ref();
        let target = victim_of(t, tm, ta, tq, terrain, self.mode_680);
        let unit = victim_of(f, fm, fa, fq, terrain, self.mode_680);
        let mut factor =
            crate::damage::elevation_factor(f.position[2], unit.attack_elevation, &target);
        if fm.kind == crate::order::BUILDING {
            factor = crate::damage::building_elevation_factor(factor, f.relics, tm.kind, t.relics);
        }
        let bonus = crate::damage::bonus_damage(&unit, &target);
        crate::damage::armor_damage(&target, &unit.weapons, factor, &unit.source, bonus)
    }

    /// What a computer player's unit weighs of object `other` on a damage notify
    /// ([`crate::unit_ai::Duel`]).
    pub(in crate::turn) fn duel(&self, other: i32) -> Result<crate::unit_ai::Duel, GenieError> {
        let (_, o, _, om) = self.held(other).ok_or(NOT_HELD)?;
        Ok(crate::unit_ai::Duel {
            converts: self.converts(other, self.me, true)?,
            attacks: attacks(om),
            harms: self.damage(other, self.me)?,
            harmed: self.damage(self.me, other)?,
            hp: o.life.hp,
        })
    }

    /// The villager's test of its attacker `other` (`FUN_140c22bf0` case 500): one of a master
    /// with object flag 0x10 (`+0x654`) that is its own copy (`+0x158` equal to `+0x58`), which
    /// shows an attack below 5 (slot 0x878, [`crate::damage::shown_attack`]; below 4, or with
    /// its master's hit points below [`HARMLESS_HP`]), a melee armor below 1 (slot 0x868) and a
    /// range of at most [`HARMLESS_RANGE`] (slot 0x880, `COMISS` with `JNC`: not a NaN), is
    /// fought back as Gaia's attackers are. Its attack's target (an Attack's, `FUN_14107cc30`)
    /// counts when the world holds it.
    pub(in crate::turn) fn harmless(&self, other: i32) -> Result<bool, GenieError> {
        let (q, o, attributes, om) = self.held(other).ok_or(NOT_HELD)?;
        if om.object_flags & 0x10 == 0 || om.copy_id != om.id {
            return Ok(false);
        }
        let terrain = self.spatial.terrain.as_ref();
        let v = victim_of(o, om, attributes, q, terrain, self.mode_680);
        let target = (o.top_action_kind() == Some(crate::attack::ATTACK))
            .then(|| o.top_target())
            .flatten()
            .and_then(|t| self.held(t))
            .map(|(tq, t, ta, tm)| victim_of(t, tm, ta, tq, terrain, self.mode_680));
        let shown = crate::damage::Shown {
            target: target.as_ref(),
            projectile: om
                .charge_projectile
                .and_then(|id| self.every.get(q)?.get(id))
                .map(|m| &m.weapons[..]),
            charge_event: om.charge_event,
            range_powerup: o.range_powerup,
        };
        let attack = crate::damage::shown_attack(&v, &shown);
        if 4 < attack || (3 < attack && HARMLESS_HP <= f32::from(om.hit_points)) {
            return Ok(false);
        }
        if 0 < crate::damage::shown_melee_armor(&v)? {
            return Ok(false);
        }
        Ok(crate::damage::shown_range(&v, &shown) <= HARMLESS_RANGE)
    }

    /// Slot 0x78 of object `target`'s unit AI (`FUN_140c48610`): of its attackers in their order,
    /// those the world holds, the one with the least footprint gap to it (`FUN_14118f680`,
    /// [`crate::movement::footprint_gap`]; the first of a tie, `COMISS`: a NaN gap takes none
    /// over), −1 for none; None for a target the world does not hold or without a unit AI.
    pub(in crate::turn) fn nearest_attacker_of(&self, target: i32) -> Option<i32> {
        let (_, t, _, tm) = self.held(target)?;
        let ai = t.ai.as_ref()?;
        let mut best: Option<(f32, i32)> = None;
        for &id in &ai.attackers {
            let Some((_, a, _, am)) = self.held(id) else {
                continue;
            };
            let gap = crate::movement::footprint_gap(t.position, tm.radius, a.position, am.radius);
            if best.is_none_or(|(least, _)| gap < least) {
                best = Some((gap, id));
            }
        }
        Some(best.map_or(-1, |(_, id)| id))
    }

    /// The object `id` as the attack tests read it, and whether it moves (slot 0x208 above 0).
    pub(in crate::turn) fn target(
        &self,
        id: i32,
    ) -> Result<(crate::unit_ai::Target, bool), GenieError> {
        let (owner, t) = self
            .foreign
            .find(id)
            .or_else(|| {
                self.others
                    .iter()
                    .find(|u| u.id == id)
                    .map(|u| (self.player, u))
            })
            .ok_or(NOT_HELD)?;
        let m = self
            .every
            .get(owner)
            .and_then(|ms| ms.get(t.life.master_id))
            .ok_or(GenieError::unsupported(
                "a found object of a master the slice does not have",
            ))?;
        let placed = self
            .spatial
            .get_known(id)?
            .ok_or(GenieError::unsupported("a found object without a place"))?;
        let capture = self
            .spatial
            .sight
            .as_ref()
            .and_then(|v| v.seer(id))
            .map_or(0, |s| s.capture);
        let target = crate::unit_ai::Target {
            owner,
            class: m.class,
            kind: m.kind,
            master: t.life.master_id,
            combat_level: m.combat_level,
            capture,
            has_ai: t.ai.is_some(),
            inside: t.life.inside.is_some(),
            targeting: !t.untargetable,
            attackable: t.attackable,
            desolid: t.construction.as_ref().is_some_and(|c| c.desolid),
            position: placed.position,
            radius: placed.radius,
        };
        let moving = 0.0 < t.mover.as_ref().map_or(0.0, |m| m.stats.speed);
        Ok((target, moving))
    }
}

impl crate::unit_ai::AttackPicker for WorldPicker<'_> {
    /// The unit, its master and its AI as `bestUnitToAttack` reads them: the AI's targets, order
    /// and action ([`crate::unit_ai::Seeker`]), its may-move test, the sight
    /// ([`crate::unit_ai::attack_sight`]), its ranges and speed, its owner's walls flag and
    /// targeting, the order history's weights and the retarget entries' targets.
    fn searcher(
        &mut self,
        memory: crate::unit_ai::Memory<'_>,
    ) -> Result<crate::best_target::Searcher, GenieError> {
        let seeker = self.seeker.ok_or(GenieError::unsupported(
            "an attack test outside bestUnitToAttack",
        ))?;
        let a = self.attacker;
        Ok(crate::best_target::Searcher {
            id: self.me,
            target: seeker.target,
            target_types: [seeker.target_type, seeker.last_target_type],
            order: seeker.order,
            action: seeker.action,
            master: a.master,
            class: a.class,
            kind: a.kind,
            attack_priority: a.attack_priority,
            computer: a.computer,
            walls_allowed: self.ai_settings.may_attack_walls,
            can_move: seeker.can_move(),
            sight: crate::unit_ai::attack_sight(a)?,
            range: a.range_against()?,
            weapon_range: a.range,
            min_range: a.min_range,
            speed: a.speed,
            scenario: a.scenario,
            targeting: (
                self.ai_settings.local_targeting_mode,
                self.ai_settings.targeting_factor_scalar,
            ),
            weights: crate::best_target::order_weights(memory.history, self.now),
            retries: memory.retries.iter().map(|&(id, _)| id).collect(),
            now: self.now,
        })
    }

    fn thing(&mut self, id: i32) -> Result<Option<crate::best_target::Thing>, GenieError> {
        let Some((owner, t, _, m)) = self.held(id) else {
            return Ok(None);
        };
        let placed = self
            .spatial
            .get_known(id)?
            .ok_or(GenieError::unsupported("a found object without a place"))?;
        let capture = self
            .spatial
            .sight
            .as_ref()
            .and_then(|v| v.seer(id))
            .map_or(0, |s| s.capture);
        let a = self.attacker;
        Ok(Some(crate::best_target::Thing {
            owner,
            allied: crate::tasks::allied(self.relations, self.player, owner, true),
            state: t.life.state,
            has_ai: t.ai.is_some(),
            inside: t.life.inside.is_some(),
            master: t.life.master_id,
            class: m.class,
            kind: m.kind,
            capture: capture != 0,
            targetable: !t.untargetable,
            attackable: t.attackable,
            held: t.life.amount,
            distance: crate::movement::footprint_gap(
                a.position,
                a.radius,
                placed.position,
                placed.radius,
            ),
            speed: t.mover.as_ref().map_or(0.0, |mv| mv.stats.speed),
        }))
    }

    fn can_attack(&mut self, id: i32) -> Result<bool, GenieError> {
        let seeker = self.seeker.ok_or(GenieError::unsupported(
            "an attack test outside bestUnitToAttack",
        ))?;
        let (target, _) = self.target(id)?;
        crate::unit_ai::can_attack(self.attacker, &seeker, &target)
    }

    fn attacks_neutral(&mut self, class: i16) -> Result<bool, GenieError> {
        crate::unit_ai::attacks_neutral(self.attacker, class)
    }

    fn spares_friends(&mut self, id: i32) -> Result<bool, GenieError> {
        let (target, _) = self.target(id)?;
        crate::unit_ai::spares_friends(self.attacker, &target)
    }

    /// `FUN_140c41690`: a human player's unit passes nothing over (the computer player's test
    /// never runs: its search fails at the sight).
    fn shuns(&mut self, _: i32) -> Result<bool, GenieError> {
        Ok(false)
    }

    /// What the statistics read of object `id`: its class and speed, whether it is armed (its
    /// attacks' sum, slot 0x228, above 0, or its priest's conversion test on the unit), the
    /// unit's range against it, the unit's damage per hit to it (slot 0x220: the unit's
    /// elevation factor, slot 0x1f0, and its armor, slot 0xf0, [`crate::damage::armor_damage`]),
    /// its hit points, the unit's reload time (slot 0x218) and the unit's own conversion test on
    /// it.
    fn candidate(
        &mut self,
        id: i32,
        distance: f32,
        category: i32,
    ) -> Result<crate::best_target::Candidate, GenieError> {
        let (_, t, _, m) = self.held(id).ok_or(NOT_HELD)?;
        let (_, me, _, mm) = self.held(self.me).ok_or(GenieError::unsupported(
            "a searching unit of a master the slice does not have",
        ))?;
        let damage = self.damage(self.me, id)?;
        let armed = 0.0 < attacks(m) || self.converts(id, self.me, true)?;
        Ok(crate::best_target::Candidate {
            id,
            distance,
            category,
            class: m.class,
            speed: t.mover.as_ref().map_or(0.0, |mv| mv.stats.speed),
            armed,
            reach: self.attacker.range_against()?,
            damage,
            hp: t.life.hp,
            reload: me.reload_stinger + (mm.reload - me.reload_powerup),
            workable: self.converts(self.me, id, false)?,
            current: self.seeker.is_some_and(|s| s.target == id),
        })
    }

    /// Slot 0x870's armor (`FUN_141213920` -> `FUN_1410857f0`): the highest melee armor (class
    /// 3; else the base armor or the highest armor of any class) plus the pierce armor powerup
    /// (`+0xa4c`, slot 0x710; slot 0x778's stinger `+0xb50` is 0), at least 100.
    fn armored(&mut self, id: i32) -> Result<bool, GenieError> {
        let (_, t, _, m) = self.held(id).ok_or(NOT_HELD)?;
        let class3 = m.armors.iter().filter(|a| a.0 == 3).map(|a| a.1).max();
        let armor = class3.unwrap_or_else(|| {
            m.armors.iter().fold(
                m.base_armor,
                |best, &(_, n)| if best < n { n } else { best },
            )
        });
        Ok(100 <= armor.wrapping_add(t.armor_powerups[1]))
    }

    /// Object `id`'s weapon range (slot 0x230: a combat object's master's, else 0).
    fn weapon_range(&mut self, id: i32) -> Result<f32, GenieError> {
        let (_, _, _, m) = self.held(id).ok_or(NOT_HELD)?;
        Ok(if 50 <= m.kind { m.weapon_range } else { 0.0 })
    }

    fn engages(&mut self, id: i32) -> Result<bool, GenieError> {
        let (_, _, _, m) = self.held(id).ok_or(NOT_HELD)?;
        Ok(0.0 < attacks(m) || self.converts(id, self.me, true)?)
    }

    /// The quick path (slot 0x3e0) within `range`, with its master's terrain mask and radius on
    /// x.
    fn quick_path(&mut self, id: i32, range: f32) -> Result<bool, GenieError> {
        let terrain = self
            .spatial
            .terrain
            .as_ref()
            .ok_or(crate::hull_path::NO_TERRAIN)?;
        let restriction =
            usize::try_from(self.restriction).map_err(|_| crate::hull_path::NO_RESTRICTION)?;
        let mask = terrain.restriction_mask(restriction);
        crate::hull_path::quick_reach(
            self.spatial,
            self.me,
            id,
            mask,
            self.attacker.radius[0],
            range,
        )
    }
}

/// A unit's slot 0x468 (`FUN_141157c50`, `getPatrolInformation`): its top level's path
/// (`FUN_140f960f0`) copied into a new path ([`crate::tile_path::Path::append`]: its points, its
/// length or one measured) with its flags, and that level's waypoint; None without path data or
/// a path at the top.
pub(in crate::turn) fn patrol_information(
    mover: Option<&Mover>,
) -> Option<(crate::unit_ai::PatrolPath, i32)> {
    let top = mover?.paths.last().filter(|l| l.has_path)?;
    let mut path = crate::tile_path::Path::default();
    path.append(&crate::tile_path::Path {
        points: top.points.clone(),
        length: top.length,
    });
    let saved = crate::unit_ai::PatrolPath {
        path,
        flags: top.path_flags,
    };
    Some((saved, top.waypoint))
}

impl crate::unit_ai::UnitSide for UnitView<'_> {
    fn patrol_information(&self) -> Option<(crate::unit_ai::PatrolPath, i32)> {
        patrol_information(self.unit.mover.as_ref())
    }

    /// Slot 0x278 (`FUN_141158720`, [`Spatial::point_clear`]): the unit, of its radius and its
    /// restriction's terrain classes, could stand at `point`.
    fn can_stand(&mut self, point: [f32; 2]) -> Result<bool, GenieError> {
        let unit = &*self.unit;
        let mover = unit.mover.as_ref().ok_or(GenieError::unsupported(
            "a stand test of a unit without movement state",
        ))?;
        self.spatial.point_clear(
            unit.id,
            point,
            mover.stats.radius[0],
            mover.stats.restriction_mask,
        )
    }

    /// The base handler's case 0x216: the unit's closest spot beside the foundation `site`
    /// ([`Spatial::spot_off`] with the notify's radii; none without the foundation), its height
    /// and class, its owner, the foundation and its current action's target as orders name them.
    fn off_footprint(
        &mut self,
        site: i32,
        radius: [f32; 2],
    ) -> Result<crate::unit_ai::OffFootprint, GenieError> {
        let unit = &*self.unit;
        let master = self
            .masters
            .and_then(|m| m.get(unit.life.master_id))
            .ok_or(GenieError::unsupported(
                "a unit sent off of a master the slice does not have",
            ))?;
        let (player, others, foreign) = (self.player, self.others, &self.foreign);
        let named = |id: i32| {
            let found = if id == unit.id {
                Some((player, unit))
            } else {
                others
                    .iter()
                    .find(|u| u.id == id)
                    .map(|u| (player, u))
                    .or_else(|| foreign.find(id))
            };
            found.map(|(p, u)| crate::unit_ai::Named {
                id,
                master: i32::from(u.life.master_id),
                owner: p as i32,
                vector: [u.position[0], u.position[2]],
            })
        };
        let spot = match self.spatial.get(site) {
            Some(placed) => {
                let mover = unit.mover.as_ref().ok_or(GenieError::unsupported(
                    "a unit sent off without movement state",
                ))?;
                self.spatial.spot_off(
                    unit.position,
                    (unit.id, mover.stats.radius[0]),
                    mover.stats.restriction_mask,
                    (placed.position, radius),
                    false,
                )?
            }
            None => None,
        };
        let target = unit.top_target().filter(|&t| t != -1);
        Ok(crate::unit_ai::OffFootprint {
            spot,
            z: unit.position[2],
            class: master.class,
            computer: self.computer,
            player: player as i32,
            site: (site != -1).then(|| named(site)).flatten(),
            action_target: target.and_then(named),
        })
    }

    /// `FUN_140c1c7c0`: the flood ([`crate::unit_ai::flood_search`]) from the unit's tile over the
    /// larger of its search radius (slot 0x2c8) and its line of sight, truncated, testing each tile
    /// it reaches (`FUN_140c2b950`: the first object of its tile's list, in order, of type 80 or
    /// more in state 0 of the unit's player); −1 for none.
    fn foundation_search(&mut self) -> Result<i32, GenieError> {
        let unit = &*self.unit;
        let master = self
            .masters
            .and_then(|m| m.get(unit.life.master_id))
            .ok_or(GenieError::unsupported(
                "a builder of a master the slice does not have",
            ))?;
        let r = if master.search_radius <= master.line_of_sight {
            master.line_of_sight
        } else {
            master.search_radius
        } as i32;
        let (ux, uy) = (unit.position[0] as i32, unit.position[1] as i32);
        let (player, others, masters, spatial) =
            (self.player, self.others, self.masters, &*self.spatial);
        let foundation_on = |x: i32, y: i32| -> Option<i32> {
            spatial.objects_on(x, y).find_map(|(id, p)| {
                if p.player != player {
                    return None;
                }
                let u = others.iter().find(|u| u.id == id)?;
                let kind = masters.and_then(|m| m.get(u.life.master_id))?.kind;
                (80 <= kind && u.life.state == 0).then_some(id)
            })
        };
        Ok(crate::unit_ai::flood_search((ux, uy), r, foundation_on).unwrap_or(-1))
    }

    fn call_for_help(&mut self, attacker: i32) -> Result<(), GenieError> {
        /// The classes that never help: 1 to 3, 18 to 21, 27 and 39.
        const NO_HELP_CLASSES: u64 = 0x0080_083c_000e;
        /// The siege tower master (`FUN_141194f10`'s 0x451).
        const SIEGE_TOWER: i16 = 0x451;
        /// `AskForHelp`'s reach (`DAT_142ecccc4`): the squared footprint gap of 3 tiles.
        const HELP_REACH: f32 = crate::image::unit_ai::HELP_REACH;
        let lists = self.spatial.searched().ok_or(GenieError::unsupported(
            "a search that reads the registry's last answer (before any query)",
        ))?;
        let (others, foreign, player) = (self.others, self.foreign, self.player);
        let find = |id: i32| {
            others
                .iter()
                .find(|u| u.id == id)
                .map(|u| (player, u))
                .or_else(|| foreign.find(id))
        };
        let gaia_attacker = find(attacker).is_some_and(|(owner, _)| owner == 0);
        let me = self.unit.id;
        let my_radius = self
            .masters
            .and_then(|ms| ms.get(self.unit.life.master_id))
            .map(|m| m.radius)
            .ok_or(crate::turn::UNIT_AI_MASTER)?;
        for r in lists[1].iter().chain(&lists[2]) {
            if r.id == me {
                continue;
            }
            let Some((owner, u)) = find(r.id) else {
                continue;
            };
            if u.ai.is_none() || 3 <= u.life.state {
                continue;
            }
            let m = self
                .every
                .get(owner)
                .and_then(|ms| ms.get(u.life.master_id))
                .ok_or(GenieError::unsupported(
                    "a helper of a master the slice does not have",
                ))?;
            let class = i64::from(m.class);
            let no_help = (0..=0x27).contains(&class) && (NO_HELP_CLASSES >> class) & 1 != 0;
            if no_help || m.id == SIEGE_TOWER {
                continue;
            }
            if !gaia_attacker && m.class == 4 {
                continue;
            }
            if owner != self.player && !self.spatial.allied_with(owner, self.player) {
                continue;
            }
            let gap = footprint_gap_squared(self.unit.position, my_radius, u.position, m.radius);
            if HELP_REACH < gap {
                continue;
            }
            if !self.helper_free(owner, u, m, attacker)? {
                continue;
            }
            return crate::unit_ai::helper_answers();
        }
        Ok(())
    }

    fn duel(&mut self, other: i32) -> Result<crate::unit_ai::Duel, GenieError> {
        self.picker(None).duel(other)
    }

    fn harmless(&mut self, attacker: i32) -> Result<bool, GenieError> {
        self.picker(None).harmless(attacker)
    }

    fn defended_attacker(&mut self, target: i32) -> Result<Option<i32>, GenieError> {
        Ok(self.picker(None).nearest_attacker_of(target))
    }

    fn seen_unit(&mut self, id: i32) -> Result<Option<crate::unit_ai::UnitSeen>, GenieError> {
        let Some(p) = self.spatial.get(id) else {
            return Ok(None);
        };
        let (owner, u) = self
            .others
            .iter()
            .find(|u| u.id == id)
            .map(|u| (self.player, u))
            .or_else(|| self.foreign.find(id))
            .ok_or(GenieError::unsupported("an object the slice does not hold"))?;
        let m = self
            .every
            .get(owner)
            .and_then(|ms| ms.get(u.life.master_id))
            .ok_or(crate::turn::OBJECT_MASTER)?;
        let me = self
            .masters
            .and_then(|ms| ms.get(self.unit.life.master_id))
            .ok_or(crate::turn::UNIT_AI_MASTER)?;
        Ok(Some(crate::unit_ai::UnitSeen {
            id,
            class: m.class,
            owner,
            combat_level: m.combat_level,
            state: u.life.state,
            target: u.ai.as_ref().map(|a| a.current_target),
            gap_squared: footprint_gap_squared(self.unit.position, me.radius, p.position, p.radius),
            position: p.position,
            hp: u.life.hp,
            master: u.life.master_id,
            kind: m.kind,
            main: u.annex_of,
            allied: crate::tasks::allied(self.relations, self.player, owner, true),
        }))
    }

    fn closest_resource(
        &mut self,
        search: crate::unit_ai::ResourceSearch,
    ) -> Result<Option<crate::unit_ai::Gatherable>, GenieError> {
        let mut w = ResourceWorld {
            player: self.player,
            spatial: self.spatial,
            zones: self.zones,
            foreign: self.foreign,
            every: self.every,
            masters: self.masters,
            plan: Planning {
                pathing: self.pathing,
                now: self.world_time,
                computer: self.computer,
                object: self.object,
                construction_rate: self.construction_rate,
                groups: self.groups,
            },
        };
        let (id, _) = closest_resource_of(
            self.unit,
            &mut w,
            self.others,
            (search.kind, search.cap, search.site),
            (search.current, search.min_state, search.point),
        )?;
        if id == -1 {
            return Ok(None);
        }
        let (owner, u) = self
            .others
            .iter()
            .find(|u| u.id == id)
            .map(|u| (self.player, u))
            .or_else(|| self.foreign.find(id))
            .ok_or(GenieError::unsupported(
                "a resource found that the slice does not hold",
            ))?;
        let class = self
            .every
            .get(owner)
            .and_then(|m| m.get(u.life.master_id))
            .ok_or(GenieError::unsupported(
                "a resource found of a master the slice lacks",
            ))?
            .class;
        Ok(Some(crate::unit_ai::Gatherable {
            id,
            position: u.position,
            class,
        }))
    }

    /// The player's master table (`FUN_14152fe40`, `+0x28`): the masters the slice holds of each
    /// player.
    fn master_class(&mut self, player: i32, master: i32) -> Result<Option<i16>, GenieError> {
        let class = usize::try_from(player)
            .ok()
            .and_then(|p| self.every.get(p))
            .zip(i16::try_from(master).ok())
            .and_then(|(ms, id)| ms.get(id))
            .map(|m| m.class);
        Ok(class)
    }

    fn reaches(&mut self, target: i32, range: f32) -> Result<bool, GenieError> {
        let mut plan = Planning {
            pathing: self.pathing,
            now: self.world_time,
            computer: self.computer,
            object: self.object,
            construction_rate: self.construction_rate,
            groups: self.groups,
        };
        let unit = &*self.unit;
        let walker = Me {
            id: unit.id,
            position: unit.position,
            group: unit.group.id,
            action_state: unit.top_action().map_or(0, |a| a.state),
        };
        let mover = unit.mover.as_ref().ok_or(GenieError::unsupported(
            "a reach test of a unit that does not move",
        ))?;
        mover.reaches_object(
            &walker,
            self.spatial,
            &mut plan,
            target,
            range,
            unit.life.master_id,
        )
    }

    fn closest(&mut self, search: &Search<'_>) -> Result<crate::unit_ai::Found, GenieError> {
        let seeker = match search.pick {
            crate::unit_ai::Pick::Attack { seeker, .. } => Some(seeker),
            _ => None,
        };
        let mut picker = WorldPicker {
            attacker: &self.attacker,
            seeker,
            me: self.unit.id,
            player: self.player,
            others: self.others,
            foreign: self.foreign,
            spatial: self.spatial,
            every: self.every,
            restriction: self
                .masters
                .and_then(|m| m.get(self.unit.life.master_id))
                .map_or(-1, |m| m.terrain_restriction),
            unit: &*self.unit,
            attributes: self.attributes,
            relations: self.relations,
            ai_settings: self.ai_settings,
            now: self.world_time,
            mode_680: self.mode_680,
        };
        let (others, foreign, every) = (self.others, self.foreign, self.every);
        // ClosestObject's reachability check (slot 0x3d0) walks the unit's paths
        let mut plan = Planning {
            pathing: self.pathing,
            now: self.world_time,
            computer: self.computer,
            object: self.object,
            construction_rate: self.construction_rate,
            groups: self.groups,
        };
        let unit = &*self.unit;
        let walker = Me {
            id: unit.id,
            position: unit.position,
            group: unit.group.id,
            action_state: unit.top_action().map_or(0, |a| a.state),
        };
        let (mover, master_id, spatial) =
            (unit.mover.as_ref(), unit.life.master_id, &*self.spatial);
        let mut reach = |id: i32, range: f32| {
            let m = mover.ok_or(GenieError::unsupported(
                "a reach test of a unit that does not move",
            ))?;
            m.reaches_object(&walker, spatial, &mut plan, id, range, master_id)
        };
        let moving = |id: i32| herded(others, foreign, id);
        // a class-10 searcher passes master 440 and the masters with object flag 0x10
        let shuns = |p: &Placed| {
            let m = every.get(p.player).and_then(|ms| ms.get(p.master)).ok_or(
                GenieError::unsupported("a found object of a master the slice does not have"),
            )?;
            Ok(p.master == PETARD || m.object_flags & 0x10 != 0)
        };
        // a farm's workers (`+0x644`) and its pasture (`+0x790`)
        let farm = |id: i32| {
            others
                .iter()
                .find(|u| u.id == id)
                .or_else(|| foreign.find(id).map(|(_, u)| u))
                .map(|u| (u.workers, u.pasture))
        };
        spatial.search(
            unit.id,
            self.player,
            search,
            &mut picker,
            &mut reach,
            &moving,
            &shuns,
            &farm,
        )
    }

    fn may_attack(
        &mut self,
        target: i32,
        seeker: &crate::unit_ai::Seeker,
    ) -> Result<Option<(i16, [f32; 3])>, GenieError> {
        let picker = self.picker(Some(*seeker));
        let held =
            self.foreign.find(target).is_some() || self.others.iter().any(|u| u.id == target);
        if !held {
            return Ok(None);
        }
        let (t, _) = picker.target(target)?;
        Ok(
            crate::unit_ai::can_attack(&self.attacker, seeker, &t)?
                .then_some((t.class, t.position)),
        )
    }

    /// The attack command (slot 0x338, `FUN_14120cea0`; classes 51 and 54 are not modelled) ->
    /// `FUN_141081a80`: a target the world does not hold fails; `+0x648` takes the world's time
    /// (not held); the unit's own Attack on the target answers true, forced after its slot 0x48
    /// on it ([`UnitView::attack_again`]); a villager's master switch (slot 0x90) is not modelled.
    /// Then the new
    /// Attack ([`crate::attack::Attack::command`]): the constructor takes its target (slot 0x178,
    /// the caller's) and clears the unit's `+0x900` (its target is new), the action list clears
    /// (the old actions' teardowns), and the install's start (`0x140a7f8c0`) of a unit inside
    /// another fails it at once (`set_state(13)`, [`crate::attack::Attack::fail_at_start`]); else
    /// it puts the unit on its standing sprite (slot 0x68 with `+0x80`) and runs the range test
    /// ([`crate::attack::Attack::begin`]): readyToAttack's failure
    /// before a `set_state(4)` leaves the animation at speed 1; a `MoveTo` that starts arrived
    /// shows the standing sprite; a unit that cannot move in reach goes ready (`set_state(6)`,
    /// [`crate::attack::Attack::enter_ready`]: readyToAttack again, then the swing, `set_state(7)`,
    /// when it faces its target and may attack), whose notifies join the start's. A notify posted
    /// while the AI runs (its buffer, `FUN_140c55270`) is not modelled.
    fn attack(
        &mut self,
        target: i32,
        force: bool,
        ai: &UnitAi,
    ) -> Result<crate::unit_ai::Attacked, GenieError> {
        let unit = &mut *self.unit;
        let master = self
            .masters
            .and_then(|m| m.get(unit.life.master_id))
            .ok_or(crate::turn::ATTACKER_MASTER)?;
        if matches!(master.class, 51 | 54) {
            return Err(GenieError::unsupported(
                "the attack command of a unit of class 51 or 54",
            ));
        }
        let no = crate::unit_ai::Attacked::default;
        let Some(t) = self.spatial.get_known(target)?.copied() else {
            return Ok(no());
        };
        if unit.attack.is_some_and(|a| a.target == target) {
            if force {
                self.attack_again(target, &t, master, ai)?;
            }
            return Ok(crate::unit_ai::Attacked {
                attacks: true,
                posted: Vec::new(),
            });
        }
        // a villager switches to its player's villager of its task swap group (`+0x7b8` 1: the
        // master table's `+0x298`, 83; else `+0x928`, 293) when the player has it (slot 0x90 with
        // 0: the action list clears, then the master's unit part; its owner's part after the
        // update)
        let master = if master.class == 4
            && let Some(next) = self.masters.and_then(|m| {
                m.get(if master.task_by_group == 1 {
                    VILLAGER_MALE
                } else {
                    VILLAGER_FEMALE
                })
            }) {
            let by = Releaser {
                id: unit.id,
                class: master.class,
                player: self.player,
            };
            unit.tear_down_actions(self.releases, self.builder_leaves, by)?;
            let world = crate::order::SwitchWorld {
                spatial: self.spatial,
                pathing: self.pathing,
                graphics: self.graphics,
                sprite: self.sprite,
            };
            let switched = crate::order::switch_master(unit, master, next, world)?;
            self.attribute_adds.extend(switched.leaving);
            if switched.promoted {
                self.promotions.push((unit.id, master.id, next.id));
            }
            self.attribute_adds.extend(switched.coming);
            next
        } else {
            master
        };
        let me_at = *self
            .spatial
            .get(unit.id)
            .ok_or(GenieError::unsupported("an attacker without a place"))?;
        let t_master = self
            .every
            .get(t.player)
            .and_then(|m| m.get(t.master))
            .ok_or(GenieError::unsupported(
                "an attack's target of a master the slice does not have",
            ))?;
        let (tx, ty) = (t.position[0] as i32, t.position[1] as i32);
        let visible = self.spatial.sees_tile(self.player, tx, ty)
            || (t_master.fog_visibility != 0 && self.spatial.explored(self.player, tx, ty)?);
        // the attack command's Attack (`FUN_141081a80`) takes no MoveTo sprite: the unit's
        let look = unit.abilities.look.as_ref();
        let arms = crate::attack::arms_looking(master, look, self.graphics, [-1; 4], -1);
        let standing = arms.idle_sprite.ok_or(GenieError::unsupported(
            "an attacker without its standing sprite",
        ))?;
        if unit.sprite.is_none() {
            return Err(GenieError::unsupported(
                "an attack command of a unit whose sprite the slice does not hold",
            ));
        }
        // the action list's clearing (FUN_140a98620): an Attack on another target tears down (its
        // MoveTo's stops the unit) and lets its targets go (slot 0x180), as do the others
        let by = Releaser {
            id: unit.id,
            class: master.class,
            player: self.player,
        };
        unit.tear_down_actions(self.releases, self.builder_leaves, by)?;
        let speed = unit
            .mover
            .as_ref()
            .ok_or(GenieError::unsupported(
                "an attacker without movement state",
            ))?
            .stats
            .speed;
        let e = crate::attack::engagement(Some(ai), speed, master, &me_at, &t, self.world_time)?;
        // the constructor clears +0x900 for a target other than its own (none yet)
        unit.attack_count = 0;
        if unit.life.inside.is_some() {
            let mut attack = crate::attack::Attack::own(target, &e, arms);
            attack.fail_at_start(&mut unit.sprite, &mut unit.anim_speed, self.sprite)?;
            unit.attack = Some(attack);
            self.takes.push((target, by));
            return Ok(crate::unit_ai::Attacked {
                attacks: true,
                posted: Vec::new(),
            });
        }
        crate::attack::set_sprite(&mut unit.sprite, standing, self.sprite);
        let mover = unit.mover.as_mut().expect("its speed read above");
        let crate::attack::Commanded {
            mut attack,
            arrived,
            mut posted,
            ready,
            failed,
            failed_first,
        } = crate::attack::Attack::command(
            (target, self.spatial),
            &e,
            arms,
            (visible, t.player),
            mover,
        )?;
        if failed_first {
            unit.anim_speed = Some(1.0);
        }
        if failed {
            attack.fail_at_start(&mut unit.sprite, &mut unit.anim_speed, self.sprite)?;
        }
        if ready {
            posted.extend(self.ready_at_start(&mut attack, master, ai)?);
        }
        let unit = &mut *self.unit;
        let mover = unit.mover.as_mut().expect("its speed read above");
        if arrived {
            // the MoveTo's start in state 1 (`0x140aa29e0`, not quiet): the master's standing
            // sprite with its speed, `+0x7b8` cleared; the next update's state 4 ends the MoveTo
            mover.shows_standing();
            unit.shows_standing_sprite(self.masters, self.graphics, self.sprite)?;
        }
        unit.attack = Some(attack);
        self.takes.push((target, by));
        Ok(crate::unit_ai::Attacked {
            attacks: true,
            posted,
        })
    }

    fn set_under_attack(&mut self, value: bool) {
        self.unit.under_attack = value;
    }

    fn stop(&mut self) -> Result<(), GenieError> {
        let masters = self.masters;
        let by = Releaser {
            id: self.unit.id,
            class: masters
                .and_then(|m| m.get(self.unit.life.master_id))
                .map_or(-1, |m| m.class),
            player: self.player,
        };
        let lets = (&mut *self.releases, &mut *self.builder_leaves);
        self.unit
            .stop_with(lets, by, masters, self.graphics, self.sprite)
    }

    fn cap_human_order(&mut self, cap: u32) -> Result<(), GenieError> {
        self.unit.last_human_order = self.unit.last_human_order.min(cap);
        self.spatial.set_gait(self.unit.id, self.unit.gait());
        Ok(())
    }

    fn seek_unexplored(
        &mut self,
        ai: &UnitAi,
        rng: &mut Well512,
    ) -> Result<Option<[i32; 2]>, GenieError> {
        let tc = town_centre(self.others)?;
        let (at, computer) = (self.unit.position, self.computer);
        // the search about the town centre only while the player has no scout map
        let scout_map = self.spatial.scout_map(self.player).is_some();
        let (mut area, map, line_of_sight) = self.scout_area()?;
        if !scout_map && let Some(p) = crate::explore::unexplored_area(tc, map, &mut area)? {
            return Ok(Some(p));
        }
        // none about the town centre: the unit AI's own search (slot 0x90)
        let seeker = crate::explore::Seeker {
            at,
            line_of_sight,
            order: ai.current_order,
            target_point: [ai.target_point[0], ai.target_point[1]],
            computer,
        };
        crate::explore::closest_undiscovered(&mut area, seeker, map, rng)
    }

    fn seek_beyond_sight(&mut self, rng: &mut Well512) -> Result<Option<[i32; 2]>, GenieError> {
        let (at, player) = (self.unit.position, self.player);
        let scouted = self.spatial.scout_map(player).is_some();
        let (mut area, map, line_of_sight) = self.scout_area()?;
        area.scouted = scouted;
        let found = crate::explore::beyond_sight(&mut area, at, line_of_sight, map, rng)?;
        // the point found is marked in the scout map (FUN_140f4d5e0 with 1)
        if let Some([x, y]) = found {
            self.spatial.scout(player, x, y);
        }
        Ok(found)
    }

    /// The player's scout map (`FUN_140c589c0`'s case 0x202 when the auto scout found nothing):
    /// a new `InfluenceMap` of 480 by 480 (`FUN_140f4a240`, the old one goes, `+0x1368` 1) marks
    /// every tile the player sees now (its visible map's `FUN_141285350` 0xf) with 1.
    fn make_scout_map(&mut self) -> Result<(), GenieError> {
        let size = crate::explore::SCOUT_MAP_SIZE;
        let mut m = crate::explore::InfluenceMap::new([size, size], 0);
        let [w, h] = self.spatial.map;
        for y in 0..h {
            for x in 0..w {
                if self.spatial.scout_sight(self.player, x, y) == Some(0xf) {
                    m.set(x, y, 1);
                }
            }
        }
        self.spatial.set_scout_map(self.player, m);
        Ok(())
    }

    /// The unit's slot 0x388 (`FUN_141226fc0`) on the foundation `target`: the world's object
    /// (`FUN_1415417a0`; the owner's sleeping list, which the unit does not see here, holds
    /// only objects of a type below 30, never a building), then its slot 0x820 with any action
    /// type (`0xffff`) and the switch ([`task_search`] on the target and its main building);
    /// with a task the new wait replaces the unit's actions, which the caller makes after the
    /// update ([`Wait`], [`Turns::wait_for_foundation`]).
    fn command(&mut self, command: crate::unit_ai::UnitCommand) -> Result<bool, GenieError> {
        match command {
            crate::unit_ai::UnitCommand::Repair(target) => self.repair_command(target),
            crate::unit_ai::UnitCommand::Enter(target) => self.enter_command(target),
            // slot 0x3a8 (FUN_14108d050): its Transport, made after the update, answers 1
            crate::unit_ai::UnitCommand::Unload {
                to,
                over_wall: false,
            } => {
                self.starts.push(AiStart::Transport(to));
                Ok(true)
            }
            _ => Err(crate::unit_ai::UNMODELLED_COMMAND),
        }
    }

    fn wait_solidify(&mut self, target: i32) -> Result<bool, GenieError> {
        let unit = &*self.unit;
        let (player, others, foreign) = (self.player, self.others, self.foreign);
        let find = |id: i32| {
            if id == unit.id {
                return Some((player, unit));
            }
            others
                .iter()
                .find(|u| u.id == id)
                .map(|u| (player, u))
                .or_else(|| foreign.find(id))
        };
        let (Some(t), Some(general)) = (
            task_target_with(target, 1, &find, self.every)?,
            task_target_with(target, 2, &find, self.every)?,
        ) else {
            return Ok(false);
        };
        let (masters, own) = self
            .masters
            .and_then(|m| Some((m, m.get(unit.life.master_id)?)))
            .ok_or(crate::turn::UNIT_AI_MASTER)?;
        let actor = Actor {
            id: unit.id,
            master: own,
            player,
            held: (unit.life.attribute_type, unit.life.amount),
            attribute_95: self.attributes.get(95).copied().unwrap_or(0.0),
            civ: self.civ,
        };
        let dip = SideWorld {
            player,
            relations: self.relations,
            foreign,
            drop_sites: self.drop_sites,
        };
        let Some((task, switch)) = task_search(masters, &actor, &t, &general, -1, &dip)? else {
            return Ok(false);
        };
        self.starts.push(AiStart::Wait(Wait {
            target,
            task,
            switch: switch.map(|m| m.id),
        }));
        Ok(true)
    }

    /// The soldier's `AttackObject` in a group (`FUN_140c17550`): the unit's future path data
    /// (slot 0x550), which the slice holds none of, goes; then a unit that moves (slot 0x5c8) or
    /// speeds up (slot 0x5a0: its forward acceleration above 0, `COMISS`/`JBE`) stops (slot
    /// 0x5d0).
    fn group_attack_stop(&mut self) -> Result<(), GenieError> {
        let Some(m) = self.unit.mover.as_mut() else {
            return Ok(());
        };
        let speeding = m.velocity.as_ref().is_some_and(|v| 0.0 < v.acceleration[2]);
        if m.is_moving() || speeding {
            m.stop()?;
        }
        Ok(())
    }

    /// Slot 400 at the full update's tail, after what the AI gave (made after the update,
    /// [`AiStart`], the last of them counting): a wait for a foundation holds no object, a
    /// repair its target, a master switch without a task left no action, an enter the object
    /// it enters; else the top action's target ([`Unit::top_target`]); either while the world
    /// holds it.
    fn top_target_now(&mut self) -> Result<Option<i32>, GenieError> {
        let target = match self.starts.last() {
            Some(AiStart::Wait(_)) => return Ok(None),
            Some(AiStart::Repair(
                RepairOrder::Retake(t)
                | RepairOrder::Start {
                    target: t,
                    task: Some(_),
                    ..
                },
            )) => Some(*t),
            // the Transport holds no target
            Some(
                AiStart::Repair(RepairOrder::Start { task: None, .. }) | AiStart::Transport(_),
            ) => None,
            Some(AiStart::Enter(e)) => Some(e.entered),
            None => self.unit.top_target().filter(|&t| t != -1),
        };
        Ok(target.filter(|&t| self.spatial.get(t).is_some()))
    }

    /// The object as the full update's tail reads it, found as [`UnitSide::seen_unit`] finds
    /// it.
    fn target_flags(&mut self, id: i32) -> Result<Option<crate::unit_ai::TargetFlags>, GenieError> {
        let Some(seen) = self.seen_unit(id)? else {
            return Ok(None);
        };
        let u = self
            .others
            .iter()
            .find(|u| u.id == id)
            .or_else(|| self.foreign.find(id).map(|(_, u)| u))
            .expect("seen_unit found it");
        let kind = self
            .every
            .get(seen.owner)
            .and_then(|ms| ms.get(u.life.master_id))
            .expect("seen_unit found its master")
            .kind;
        Ok(Some(crate::unit_ai::TargetFlags {
            owner: seen.owner,
            kind,
            capture: u.capture_flag,
            pasture: u.pasture,
            untargetable: u.untargetable,
            attackable: u.attackable,
        }))
    }
}

impl UnitView<'_> {
    /// The unit's repair command (slot 0x378, `FUN_141217c00`, forced) on `target`: an object the
    /// world (`FUN_1415417a0`) does not hold is 0. A Repair on top takes it (its slot 0x48,
    /// `FUN_140adb480`): 1. Else a unit of task swap group 1 or 2 switches to its player's
    /// repairer ([`crate::repair::REPAIRERS`], slot 0x90 with 0), and its slot 0x820 with the
    /// repair's action type (`0x6a`) and the switch ([`task_search`] on the target and its main
    /// building, the new master's list first) finds a task: a new Repair on it
    /// (`FUN_140ad7fa0`) replaces the unit's actions (slot 0x808): 1; none: 0. The caller makes
    /// the switches and the Repair after the update ([`RepairOrder`], [`Turns::repair_order`]).
    fn repair_command(&mut self, target: i32) -> Result<bool, GenieError> {
        let unit = &*self.unit;
        let (player, others, foreign) = (self.player, self.others, self.foreign);
        let find = |id: i32| {
            if id == unit.id {
                return Some((player, unit));
            }
            others
                .iter()
                .find(|u| u.id == id)
                .map(|u| (player, u))
                .or_else(|| foreign.find(id))
        };
        if find(target).is_none() {
            return Ok(false);
        }
        if unit.top_action_kind() == Some(crate::repair::REPAIR) {
            self.starts
                .push(AiStart::Repair(RepairOrder::Retake(target)));
            return Ok(true);
        }
        let masters = self.masters.ok_or(crate::turn::UNIT_AI_MASTER)?;
        let own = masters
            .get(unit.life.master_id)
            .ok_or(crate::turn::UNIT_AI_MASTER)?;
        let switch = crate::repair::REPAIRERS
            .iter()
            .find(|&&(group, _)| group == own.task_by_group)
            .and_then(|&(_, id)| masters.get(id));
        let (Some(t), Some(general)) = (
            task_target_with(target, 1, &find, self.every)?,
            task_target_with(target, 2, &find, self.every)?,
        ) else {
            return Err(GenieError::unreachable(
                "a repair command's target the world holds without its task target",
            ));
        };
        let actor = Actor {
            id: unit.id,
            master: switch.unwrap_or(own),
            player,
            held: (unit.life.attribute_type, unit.life.amount),
            attribute_95: self.attributes.get(95).copied().unwrap_or(0.0),
            civ: self.civ,
        };
        let dip = SideWorld {
            player,
            relations: self.relations,
            foreign,
            drop_sites: self.drop_sites,
        };
        let found = task_search(masters, &actor, &t, &general, crate::repair::REPAIR, &dip)?;
        if switch.is_none() && found.is_none() {
            return Ok(false);
        }
        self.starts.push(AiStart::Repair(RepairOrder::Start {
            target,
            switches: [
                switch.map(|m| m.id),
                found.and_then(|(_, m)| m.map(|m| m.id)),
            ],
            task: found.map(|(task, _)| task),
        }));
        Ok(found.is_some())
    }

    /// The unit's enter command (slot 0x3a0, `FUN_1410847e0`, forced) on `target`: an object the
    /// world (`FUN_1415417a0`) does not hold is 0; its slot 0x820 with the garrison filter (3)
    /// and the switch ([`task_search`] on the target and its main building) finds a task, whose
    /// Enter (`FUN_140a855d0`) replaces the unit's actions (an Artifact on top would take it as
    /// its sub-action): 1; none: 0. The caller makes the switch and the Enter after the update
    /// ([`EnterOrder`], [`Turns::enter_order`]).
    fn enter_command(&mut self, target: i32) -> Result<bool, GenieError> {
        let unit = &*self.unit;
        let (player, others, foreign) = (self.player, self.others, self.foreign);
        let find = |id: i32| {
            if id == unit.id {
                return Some((player, unit));
            }
            others
                .iter()
                .find(|u| u.id == id)
                .map(|u| (player, u))
                .or_else(|| foreign.find(id))
        };
        let Some((tp, t_unit)) = find(target) else {
            return Ok(false);
        };
        let under_artifact = unit.top_action_kind() == Some(crate::artifact::ARTIFACT_TYPE);
        let masters = self.masters.ok_or(crate::turn::UNIT_AI_MASTER)?;
        let own = masters
            .get(unit.life.master_id)
            .ok_or(crate::turn::UNIT_AI_MASTER)?;
        let (Some(t), Some(general)) = (
            task_target_with(target, 1, &find, self.every)?,
            task_target_with(target, 2, &find, self.every)?,
        ) else {
            return Err(GenieError::unreachable(
                "an enter command's target the world holds without its task target",
            ));
        };
        let actor = Actor {
            id: unit.id,
            master: own,
            player,
            held: (unit.life.attribute_type, unit.life.amount),
            attribute_95: self.attributes.get(95).copied().unwrap_or(0.0),
            civ: self.civ,
        };
        let dip = SideWorld {
            player,
            relations: self.relations,
            foreign,
            drop_sites: self.drop_sites,
        };
        let Some((task, switch)) =
            task_search(masters, &actor, &t, &general, crate::order::GARRISON, &dip)?
        else {
            return Ok(false);
        };
        // the constructor's target: a building's annex enters its main building
        let building = self
            .every
            .get(tp)
            .and_then(|m| m.get(t_unit.life.master_id))
            .is_some_and(|m| m.kind == crate::order::BUILDING);
        let entered = if building {
            t_unit.annex_of.unwrap_or(target)
        } else {
            target
        };
        self.starts.push(AiStart::Enter(EnterOrder {
            target,
            entered,
            task,
            switch: switch.map(|m| m.id),
            under_artifact,
        }));
        Ok(true)
    }

    /// What an explorer's searches ask of the world, the map's size and the unit's line of sight.
    pub(in crate::turn) fn scout_area(
        &mut self,
    ) -> Result<(ScoutArea<'_>, [i32; 2], f32), GenieError> {
        let unit = &*self.unit;
        let master = self
            .masters
            .and_then(|m| m.get(unit.life.master_id))
            .ok_or(GenieError::unsupported(
                "an auto scout of a master the slice does not have",
            ))?;
        let mover = unit.mover.as_ref().ok_or(GenieError::unsupported(
            "an auto scout without movement state",
        ))?;
        let [w, h] = self.spatial.map;
        let terrain = self
            .spatial
            .terrain
            .as_ref()
            .ok_or(GenieError::unsupported(
                "the map's zones without its terrain",
            ))?;
        let values = terrain
            .restrictions
            .get(mover.stats.restriction)
            .ok_or(crate::zones::UNKNOWN_RESTRICTION)?;
        let zones = self.zones.of(w, h, &terrain.kinds, values);
        let me = Me {
            id: unit.id,
            position: unit.position,
            group: unit.group.id,
            action_state: unit.top_action().map_or(0, |a| a.state),
        };
        let area = ScoutArea {
            spatial: self.spatial,
            zones,
            plan: Planning {
                pathing: self.pathing,
                now: self.world_time,
                computer: self.computer,
                object: self.object,
                construction_rate: self.construction_rate,
                groups: self.groups,
            },
            mover,
            me,
            player: self.player,
            master: master.id,
            radius: master.radius,
            reach: master.line_of_sight - LOS_MARGIN,
            scouted: false,
        };
        Ok((area, [w, h], master.line_of_sight))
    }
}

impl Context<'_> {
    /// A walk and the Sprite stream, for an attacker's update.
    pub(in crate::turn) fn walk_and_sprites<'b>(
        &'b mut self,
        mover: Option<&'b mut Mover>,
        me: Me,
    ) -> (Walk<'b>, &'b mut Well512) {
        let (object, sprite) = self.streams.two(Stream::Object, Stream::Sprite);
        (
            Walk {
                mover,
                me,
                dt: self.dt_seconds,
                world: self.spatial,
                plan: Some(Planning {
                    pathing: self.pathing,
                    now: self.world_time,
                    computer: self.computer,
                    object,
                    construction_rate: self.construction_rate,
                    groups: self.groups,
                }),
            },
            sprite,
        )
    }

    /// A walk with the Sprite and the Action streams (a farmer's stops draw from the second).
    pub(in crate::turn) fn walk_sprites_action<'b>(
        &'b mut self,
        mover: Option<&'b mut Mover>,
        me: Me,
    ) -> (Walk<'b>, &'b mut Well512, &'b mut Well512) {
        let [object, sprite, action] =
            self.streams
                .three(Stream::Object, Stream::Sprite, Stream::Action);
        (
            Walk {
                mover,
                me,
                dt: self.dt_seconds,
                world: self.spatial,
                plan: Some(Planning {
                    pathing: self.pathing,
                    now: self.world_time,
                    computer: self.computer,
                    object,
                    construction_rate: self.construction_rate,
                    groups: self.groups,
                }),
            },
            sprite,
            action,
        )
    }

    /// A walk and another stream than the walk's Object stream.
    pub(in crate::turn) fn walk_with<'b>(
        &'b mut self,
        mover: Option<&'b mut Mover>,
        me: Me,
        other: Stream,
    ) -> (Walk<'b>, &'b mut Well512) {
        let (object, other) = self.streams.two(Stream::Object, other);
        (
            Walk {
                mover,
                me,
                dt: self.dt_seconds,
                world: self.spatial,
                plan: Some(Planning {
                    pathing: self.pathing,
                    now: self.world_time,
                    computer: self.computer,
                    object,
                    construction_rate: self.construction_rate,
                    groups: self.groups,
                }),
            },
            other,
        )
    }

    /// What an object's walking action needs from the tick.
    pub(in crate::turn) const fn walk<'b>(
        &'b mut self,
        mover: Option<&'b mut Mover>,
        me: Me,
    ) -> Walk<'b> {
        Walk {
            mover,
            me,
            dt: self.dt_seconds,
            world: self.spatial,
            plan: Some(Planning {
                pathing: self.pathing,
                now: self.world_time,
                computer: self.computer,
                object: self.streams.get(Stream::Object),
                construction_rate: self.construction_rate,
                groups: self.groups,
            }),
        }
    }
}

/// The auto scout's search reaches within its line of sight less this (`FUN_140c65490`,
/// `DAT_14330eb8c`).
pub(in crate::turn) const LOS_MARGIN: f32 = crate::image::explore::LOS_MARGIN;

/// The masters of a town centre (`FUN_1416597e0`: 109, then 444).
pub(in crate::turn) const TOWN_CENTRES: [i16; 2] = [109, 444];

/// `FUN_1416597e0` with 0: the player's first town centre, by the player's index of objects by
/// master (`player+0x18`, `FUN_14104f560`: those of master 109, then 444), as its tile. The
/// index's order within a master is not held: two objects of the first master found are not
/// modelled.
pub(in crate::turn) fn town_centre(units: &[Unit]) -> Result<Option<[i32; 2]>, GenieError> {
    for master in TOWN_CENTRES {
        let mut found = units.iter().filter(|u| u.life.master_id == master);
        let Some(first) = found.next() else { continue };
        if found.next().is_some() {
            return Err(GenieError::unsupported(
                "two town centres (the order of the player's index)",
            ));
        }
        return Ok(Some([first.position[0] as i32, first.position[1] as i32]));
    }
    Ok(None)
}

/// What the auto scout's search asks of the world for one unit ([`crate::explore::Area`]).
pub(in crate::turn) struct ScoutArea<'a> {
    pub(in crate::turn) spatial: &'a Spatial,
    pub(in crate::turn) zones: &'a crate::zones::ZoneMap,
    pub(in crate::turn) plan: Planning<'a>,
    pub(in crate::turn) mover: &'a Mover,
    pub(in crate::turn) me: Me,
    pub(in crate::turn) player: usize,
    pub(in crate::turn) master: i16,
    pub(in crate::turn) radius: [f32; 2],
    /// The reach of slot 0x3d8: the line of sight less a tile.
    pub(in crate::turn) reach: f32,
    /// The search reads the player's scout map for the tiles seen (`FUN_140c4b130` with one).
    pub(in crate::turn) scouted: bool,
}

impl crate::explore::Area for ScoutArea<'_> {
    fn explored(&self, x: i32, y: i32) -> Result<bool, GenieError> {
        if self.scouted {
            return self
                .spatial
                .scout_map(self.player)
                .and_then(|m| m.get(x, y))
                .map(|v| v == 1)
                .ok_or(GenieError::unsupported("a tile off the player's scout map"));
        }
        self.spatial.explored(self.player, x, y)
    }

    fn zone(&mut self, x: i32, y: i32) -> u16 {
        self.zones.zone(x, y)
    }

    fn own_zone(&mut self) -> u16 {
        let [x, y, _] = self.me.position;
        self.zones.zone(x as i32, y as i32)
    }

    fn clear(&self, x: i32, y: i32) -> Result<bool, GenieError> {
        let mask = self.mover.stats.restriction_mask;
        self.spatial
            .point_clear(self.me.id, [x as f32, y as f32], self.radius[0], mask)
    }

    fn reaches(&mut self, x: i32, y: i32) -> Result<bool, GenieError> {
        let to = [x as f32, y as f32];
        self.mover.reaches(
            &self.me,
            self.spatial,
            &mut self.plan,
            to,
            self.reach,
            self.master,
        )
    }

    /// `FUN_14118faf0`: the gap from the footprint on each axis (at least 0), then its length.
    fn distance(&self, x: i32, y: i32) -> f32 {
        let gap = |d: f32, r: f32| {
            let g = d.abs() - r;
            if g <= 0.0 { 0.0 } else { g }
        };
        let gx = gap(x as f32 - self.me.position[0], self.radius[0]);
        let gy = gap(y as f32 - self.me.position[1], self.radius[1]);
        crate::fmath::sqrt(gy * gy + gx * gx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Slot 0x228 (`FUN_141084070`): the attacks above 0 add up in their order from +0; none
    /// (or none above 0) is +0.
    #[test]
    fn the_attacks_sum_is_the_positive_ones() {
        let m = |weapons: Vec<(i16, i16)>| crate::masters::Master {
            weapons,
            ..crate::masters::Master::default()
        };
        assert_eq!(
            attacks(&m(vec![(4, 6), (3, -2), (11, 1)])).to_bits(),
            7.0f32.to_bits()
        );
        assert_eq!(attacks(&m(vec![])).to_bits(), 0.0f32.to_bits());
        assert_eq!(
            attacks(&m(vec![(4, 0), (3, -5)])).to_bits(),
            0.0f32.to_bits()
        );
    }
    /// The villager 123 (player 1) tests player 2's unit 50 of `master`: harmless when its
    /// master has object flag 0x10, is its own copy and shows an attack below 5 (4 only below 60
    /// hit points), no melee armor and a range of at most 0.5.
    fn harmless(master: crate::masters::Master) -> bool {
        let me = Unit {
            id: 123,
            life: crate::lifecycle::Lifecycle {
                master_id: 83,
                ..crate::lifecycle::Lifecycle::default()
            },
            ..Unit::default()
        };
        let enemy = Player {
            objects: vec![Unit {
                id: 50,
                life: crate::lifecycle::Lifecycle {
                    master_id: master.id,
                    state: 2,
                    ..crate::lifecycle::Lifecycle::default()
                },
                ..Unit::default()
            }],
            ..Player::default()
        };
        let (gaia, after) = (Player::default(), [enemy]);
        let before = [gaia];
        let every = [
            Masters::default(),
            Masters::default(),
            Masters::new([master]),
        ];
        let spatial = Spatial::new([120, 120]);
        let attacker = crate::unit_ai::Attacker {
            ai_type: 8,
            player: 1,
            computer: false,
            state: 2,
            held: -1,
            top_action: None,
            master: 83,
            kind: 70,
            class: 4,
            attack_priority: 0,
            missile: -1,
            range: 0.0,
            min_range: 0.0,
            charge_type: 0,
            area_effect: 0.0,
            blast_level: 0,
            los: 4.0,
            position: [10.0, 10.0, 0.0],
            radius: [0.2, 0.2],
            speed: 0.8,
            scenario: false,
            firepower: 0.0,
            capture: 0,
        };
        let picker = WorldPicker {
            attacker: &attacker,
            seeker: None,
            me: 123,
            player: 1,
            others: &[],
            foreign: Foreign {
                before: &before,
                after: &after,
                left: None,
            },
            spatial: &spatial,
            every: &every,
            restriction: -1,
            unit: &me,
            attributes: &[],
            relations: &[0, 0, 0],
            ai_settings: crate::turn::player::AiSettings::default(),
            now: 0,
            mode_680: 0,
        };
        picker.harmless(50).unwrap()
    }

    /// Slot 0x78 of the ward's unit AI: of its attackers the world holds, the nearest by the
    /// footprint gap, the first of a tie; −1 for none, None for a ward without a unit AI.
    #[test]
    fn a_ward_s_nearest_attacker_is_its_closest_held_one() {
        let master = |id, radius| crate::masters::Master {
            id,
            kind: 70,
            radius,
            ..crate::masters::Master::default()
        };
        let unit = |id, x, master_id, ai: Option<UnitAi>| Unit {
            id,
            position: [x, 10.0, 0.0],
            ai,
            life: crate::lifecycle::Lifecycle {
                master_id,
                state: 2,
                ..crate::lifecycle::Lifecycle::default()
            },
            ..Unit::default()
        };
        let attacked = |attackers| UnitAi {
            attackers,
            ..UnitAi::default()
        };
        let me = unit(123, 10.0, 4, None);
        let others = [
            unit(20, 12.0, 4, Some(attacked(vec![50, 51, 52, 53]))),
            unit(21, 12.0, 4, None),
            unit(22, 12.0, 4, Some(attacked(vec![99]))),
        ];
        let enemy = Player {
            // 50 at a gap of 4.4, 51 of 2.4 (its radius 1), 53 of 2.4 too
            objects: vec![
                unit(50, 17.0, 5, None),
                unit(51, 16.0, 6, None),
                unit(53, 16.0, 6, None),
            ],
            ..Player::default()
        };
        let (gaia, after) = (Player::default(), [enemy]);
        let before = [gaia];
        let every = [
            Masters::default(),
            Masters::new([master(4, [0.3, 0.3])]),
            Masters::new([master(5, [0.3, 0.3]), master(6, [1.3, 0.3])]),
        ];
        let spatial = Spatial::new([120, 120]);
        let attacker = crate::unit_ai::Attacker {
            ai_type: 13,
            player: 1,
            computer: false,
            state: 2,
            held: -1,
            top_action: None,
            master: 4,
            kind: 70,
            class: 6,
            attack_priority: 0,
            missile: -1,
            range: 0.0,
            min_range: 0.0,
            charge_type: 0,
            area_effect: 0.0,
            blast_level: 0,
            los: 4.0,
            position: [10.0, 10.0, 0.0],
            radius: [0.3, 0.3],
            speed: 1.0,
            scenario: false,
            firepower: 0.0,
            capture: 0,
        };
        let picker = WorldPicker {
            attacker: &attacker,
            seeker: None,
            me: 123,
            player: 1,
            others: &others,
            foreign: Foreign {
                before: &before,
                after: &after,
                left: None,
            },
            spatial: &spatial,
            every: &every,
            restriction: -1,
            unit: &me,
            attributes: &[],
            relations: &[0, 0, 0],
            ai_settings: crate::turn::player::AiSettings::default(),
            now: 0,
            mode_680: 0,
        };
        assert_eq!(picker.nearest_attacker_of(20), Some(51));
        assert_eq!(picker.nearest_attacker_of(21), None);
        assert_eq!(picker.nearest_attacker_of(22), Some(-1));
        assert_eq!(picker.nearest_attacker_of(77), None);
    }

    #[test]
    fn a_villager_fights_back_a_weak_attacker_of_its_own_master() {
        let villager = crate::masters::Master {
            id: 83,
            copy_id: 83,
            kind: 70,
            class: 4,
            object_flags: 0x10,
            hit_points: 25,
            weapons: vec![(4, 3), (21, 6)],
            armors: vec![(4, 0), (3, 0)],
            weapon_range: 0.0,
            ..crate::masters::Master::default()
        };
        assert!(harmless(villager.clone()));
        let with = |f: &dyn Fn(&mut crate::masters::Master)| {
            let mut m = villager.clone();
            f(&mut m);
            harmless(m)
        };
        assert!(!with(&|m| m.object_flags = 0));
        assert!(!with(&|m| m.copy_id = 293));
        assert!(!with(&|m| m.weapons = vec![(4, 5)]));
        // an attack of 4 below 60 hit points
        assert!(with(&|m| m.weapons = vec![(4, 4)]));
        assert!(!with(&|m| (m.weapons, m.hit_points) = (vec![(4, 4)], 60)));
        assert!(with(&|m| (m.weapons, m.hit_points) = (vec![(4, 3)], 60)));
        assert!(!with(&|m| m.armors = vec![(4, 1)]));
        assert!(with(&|m| m.weapon_range = 0.5));
        assert!(!with(&|m| m.weapon_range = 0.6));
        assert!(!with(&|m| m.weapon_range = f32::NAN));
    }
}
