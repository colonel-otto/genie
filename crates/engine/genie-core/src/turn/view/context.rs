//! What a tick's update of one object reads and writes ([`Context`]) and its walks.

use crate::combat::damage::Victim;
use crate::construction::build::Walk;
use crate::masters::Masters;
use crate::math::rng::{Stream, Streams, Well512};
use crate::movement::{Me, Mover, Planning};
use crate::pathing::Pathing;
use crate::spatial::Spatial;
use crate::sprite::Graphic;
use crate::tasks::DropSite;
use crate::turn::unit::{AiResume, Unit};
use crate::turn::view::{AiStart, AiWork, Foreign, RepairWrite};
use crate::turn::world::Releaser;
use crate::turn::{BuildingDeath, Recycling};
use crate::unit_ai::{AiMove, GenieError};

pub(crate) struct Context<'a> {
    /// The tick's dt in milliseconds, and in seconds (`world+0x164`).
    pub(in crate::turn) dt: u32,
    pub(crate) dt_seconds: f32,
    /// The ticks so far, this one included.
    pub(in crate::turn) tick: u32,
    /// The owner's accumulated time when it has the turn: the full unit-AI update runs.
    pub(in crate::turn) turn: Option<u32>,
    pub(crate) world_time: u32,
    pub(crate) computer: bool,
    pub(crate) streams: &'a mut Streams,
    /// Every object's place, which a walker reads and its moves update.
    pub(crate) spatial: &'a mut Spatial,
    /// The planners and the mesh, which a walker's path request uses.
    pub(crate) pathing: &'a mut Pathing,
    /// The world's unit groups, which a walker's long path reads.
    pub(crate) groups: &'a crate::group::UnitGroups,
    /// The owner's construction rate (attribute 195).
    pub(crate) construction_rate: f32,
    /// The owner's masters, which an attack reads.
    pub(crate) masters: Option<&'a Masters>,
    /// The target of the unit's attack, read before its update and written back after it, and
    /// the objects about a unit with an area effect, which its blast may reach.
    pub(crate) victim: Option<&'a mut Victim>,
    /// The unit AI's current target as its target block reads it ([`Turns::seen`]).
    pub(in crate::turn) ai_target: Result<Option<crate::unit_ai::Seen>, GenieError>,
    pub(crate) blasted: Vec<Victim>,
    /// The unit was killed in its update (a retaliation), which the caller handles after it.
    pub(in crate::turn) own_killed: Option<crate::combat::damage::Kill>,
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
    pub(crate) gate_toggle: bool,
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
    pub(crate) releases: Vec<(i32, Releaser)>,
    pub(crate) takes: Vec<(i32, Releaser)>,
    /// The foundations a counted builder's Build leaves as the unit's action list goes
    /// ([`Unit::clear_own_actions`], `FUN_140aadbf0`): the building `FUN_140aaddb0` gives counts one
    /// builder less ([`Turns::leave_builders`]), the caller's before the releases.
    pub(in crate::turn) builder_leaves: Vec<i32>,
    /// The unit group a unit that entered a building left (`FUN_1411921d0`), with the
    /// building: the caller runs the group's removeUnit ([`Turns::remove_unit`]).
    pub(in crate::turn) left_group: Option<(i32, i32)>,
    /// The objects the unit made (a ranged unit's missiles), which join the end of its owner's
    /// list after it and update later in the same tick.
    pub(crate) made: Vec<Unit>,
    /// The objects whose master leaves ghosts that died in this update ([`crate::vision::ghost`]), the
    /// caller's.
    pub(in crate::turn) ghost_deaths: Vec<i32>,
    /// The buildings whose construction started in this update: their owner watches them
    /// ([`Turns::watch_started`]), the caller's.
    pub(crate) started: Vec<i32>,
    /// The buildings that died in this update: their owner's watched list lets them go
    /// (`FUN_141091310`), the caller's.
    pub(in crate::turn) unwatched: Vec<i32>,
    /// The annexes a building's `set_state` (`FUN_1411dbb70`) took to its own state (slot 0xa0
    /// on each, before its own parts), the caller's after the update ([`Turns::set_state`]).
    pub(crate) annex_states: Vec<(i32, u8)>,
    /// The annexes a dying building let go (the static update's death, `0x14118ad50`), each with
    /// the hit points it takes: its `+0xbb8` clears, the caller's after the update.
    pub(in crate::turn) released_annexes: Vec<(i32, f32)>,
    /// The objects a builder's footprint clearing deletes (the Build's `set_state(7)`,
    /// `FUN_1415c3620` with `param_6`; their records went already), the caller's after the
    /// update ([`Turns::clear_object`]).
    pub(in crate::turn) cleared: Vec<crate::construction::build::Cleared>,
    /// The tracker's promotions (`FUN_14104fb20`) of the owner's units whose master changed
    /// while the unit AI ran (the unit, its old master, its new one), the caller's after the
    /// update ([`Turns::track_promote`]).
    pub(in crate::turn) promotions: Vec<(i32, i16, i16)>,
    /// The flares a hit on a player's object has that player make (master 112, its slot 0x1c0)
    /// at the attacker's place, doubled in their amount for a trebuchet's hit, the caller's
    /// after the update ([`Turns::make_flare`]).
    pub(crate) flares: Vec<(usize, [f32; 3], bool)>,
    /// The foundations another player's marks lie on, which a builder's `set_state(7)` removes
    /// when their owner is human ([`crate::construction::build::BuildStep::removes`]), the caller's after the
    /// update ([`Turns::remove_building`]).
    pub(in crate::turn) site_removals: Vec<i32>,
    /// The units on a foundation's footprint its builder's test met (the foundation, them):
    /// the caller sends off those the test would ([`Turns::evacuate`]).
    pub(in crate::turn) evacuations: Vec<(i32, Vec<i32>)>,
    /// The world's reusable ids and pools.
    pub(crate) recycling: &'a mut Recycling,
    /// The game's setting at `+0x680` is 1 ([`Turns::mode_680`]).
    pub(in crate::turn) mode_1: bool,
    /// The owner's attributes, which a gatherer's productivity reads.
    pub(crate) attributes: &'a [f32],
    /// The owner's food, wood, stone and gold ([`Player::resources`]), which a repair pays from.
    ///
    /// [`Player::resources`]: crate::turn::player::Player::resources
    pub(crate) resources: [f32; 4],
    /// The graphics, for the sprites a hunter takes.
    pub(crate) graphics: &'a std::collections::BTreeMap<i16, Graphic>,
    /// The drop-site table, whose livestock entries a hunt leads its livestock to.
    pub(crate) drop_sites: &'a [DropSite],
    /// The owner's id and civ (`player+0x738`).
    pub(crate) player: usize,
    pub(crate) civ: u8,
    /// The owner's AI settings (`+0x608`..), which its unit AIs read.
    pub(in crate::turn) ai_settings: crate::turn::player::AiSettings,
    /// The player shares its gathered resources with its allies (`+0x1426`).
    pub(crate) sharing: bool,
    /// The game's farm setting ([`crate::settings::Settings::walkable_farms`]).
    pub(crate) farm_stops: bool,
    /// The game's setting [`crate::settings::Settings::predator_hunts_end`].
    pub(in crate::turn) predator_hunts_end: bool,
    /// Every player's masters and attribute 97 (by player), for a capture.
    pub(crate) every_masters: &'a [Masters],
    pub(in crate::turn) attribute_97: &'a [f32],
    /// Every player's game status (`+0x89`), which the Artifact's owner search reads.
    pub(in crate::turn) game_status: &'a [u8],
    /// The player whose turn this tick is (`world+0x188`), every player's accumulated time
    /// (`world+0x18c`, as it stands when the object updates) and computer flag.
    pub(in crate::turn) player_turn: usize,
    pub(in crate::turn) player_time: &'a [u32],
    pub(crate) computers: &'a [bool],
    /// Each player's `chance_to_dodge_missiles` ([`crate::turn::Player::dodge_chance`]).
    pub(in crate::turn) dodge_chances: &'a [u8],
    /// Each player's `chance_for_archers_to_maintain_distance`
    /// ([`crate::turn::Player::keep_distance_chance`]).
    pub(in crate::turn) keep_distance_chances: &'a [u8],
    /// Some player's attribute 204 is not 0, which a building's completion reads.
    pub(crate) attribute_204: bool,
    /// [`Turns::mode_680`].
    pub(crate) mode_680: i32,
    /// The game's carry rule ([`crate::settings::Settings::carry_rule`]), which a hunt's start
    /// reads.
    pub(in crate::turn) carry_rule: bool,
    /// The game mode is one of `FUN_141302540`'s ([`scenario_mode`]).
    pub(crate) scenario: bool,
    /// The game mode's descriptor flag `+0x9c` ([`standard_mode`]) and each player's computer
    /// flag and `mAllowCapturingGaiaUnits`, which a takeover by sight reads
    /// ([`crate::vision::visibility::sight_taker`]).
    pub(in crate::turn) standard: bool,
    pub(in crate::turn) sight_takers: &'a [(bool, bool)],
    /// The owner's object tracker, which a completion's first part (`FUN_1411c6bd0`) moves a
    /// building in, and the unit lines its keys read ([`crate::tracker`]).
    pub(crate) tracker: &'a mut crate::tracker::Tracker,
    pub(crate) lines: &'a crate::lines::UnitLines,
    /// The attributes the owner gains from the object's update (a building it completed), which
    /// the caller adds after it.
    pub(crate) attribute_adds: Vec<(i16, f32)>,
    /// The events the owner's history records from the object's update (a castle or a wonder it
    /// completed, [`crate::construction::build::Built::history`]), which the caller adds after it.
    pub(in crate::turn) history_events: Vec<u8>,
    /// The technologies the owner researches from the object's update (a completed building's),
    /// which the caller completes after the attributes ([`Turns::complete_research`]).
    pub(crate) techs_done: Vec<i16>,
    /// The pastures the object's update completed, whose livestock the caller makes after the
    /// attributes ([`Turns::setup_pasture`]).
    pub(crate) pastures: Vec<i32>,
    /// The mule carts the object's update completed, whose search for resources the caller runs
    /// after it ([`Turns::cart_search`]).
    pub(crate) carts: Vec<(i32, u8)>,
    /// The buildings the object's update completed with a stack unit or that disappear when
    /// built: the caller makes the stack unit ([`Turns::stack_unit`]) before the technologies
    /// and destroys the building (slot 0xd0) after them.
    pub(crate) completions: Vec<(i32, Option<crate::construction::build::StackUnit>, bool)>,
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
    pub(crate) build_command: Option<i32>,
    /// A Gather's walk back to reseed ended (state 0xf2, whether it arrived), which the caller
    /// reseeds after the update ([`Turns::reseed_after_walk`]).
    pub(crate) reseed: Option<bool>,
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
    /// seconds; [`crate::construction::build::Worked::Main`]), which the caller runs after the update
    /// ([`Turns::work_building`]).
    pub(in crate::turn) main_works: Vec<(i32, f32)>,
    /// What a repair changed of another player's objects ([`RepairWrite`]), which the caller
    /// writes after the update.
    pub(crate) repair_writes: Vec<RepairWrite>,
    /// The owner's diplomacy (`player+0x260`), which the unit's task search reads.
    pub(crate) relations: &'a [u8],
    /// The livestock a hunt sends to a drop site with its herd, which the caller sends after the
    /// update ([`Turns::lead_herd`]).
    pub(in crate::turn) leads: Option<(i32, i32)>,
    /// The map's zone maps, which an auto scout's search reads.
    pub(crate) zones: &'a mut crate::pathing::zones::Zones,
    /// The other players' lists, as they stand before the unit's update.
    pub(crate) foreign: Foreign<'a>,
}

/// What an object's step gathered in its Context for the world to apply after it
/// ([`crate::turn::Turns::finish_object`]): the Context's fields of the same meaning; the left
/// group carries the unit's id.
pub(crate) struct ObjectOutputs {
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
    /// [`Context`]'s `pastures`.
    pub(in crate::turn) pastures: Vec<i32>,
    /// [`Context`]'s `carts`.
    pub(in crate::turn) carts: Vec<(i32, u8)>,
    /// [`Context`]'s `completions`.
    pub(in crate::turn) completions:
        Vec<(i32, Option<crate::construction::build::StackUnit>, bool)>,
    /// [`Context`]'s `work_order`.
    pub(in crate::turn) work_order: Option<crate::order::Work>,
    /// [`Context`]'s `transport_unload`.
    pub(in crate::turn) transport_unload: bool,
    /// [`Context`]'s `blasted`.
    pub(in crate::turn) blasted: Vec<Victim>,
    /// [`Context`]'s `own_killed`.
    pub(in crate::turn) own_killed: Option<crate::combat::damage::Kill>,
    /// [`Context`]'s `remains`.
    pub(in crate::turn) remains: Option<(i16, [f32; 3], Option<u8>)>,
    /// [`Context`]'s `refund`.
    pub(in crate::turn) refund: Vec<(i16, f32)>,
    /// [`Context`]'s `annex_states`.
    pub(in crate::turn) annex_states: Vec<(i32, u8)>,
    /// [`Context`]'s `cleared`.
    pub(in crate::turn) cleared: Vec<crate::construction::build::Cleared>,
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
            pastures: std::mem::take(&mut self.pastures),
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
    pub(crate) fn walk_sprites_action<'b>(
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
    pub(crate) const fn walk<'b>(&'b mut self, mover: Option<&'b mut Mover>, me: Me) -> Walk<'b> {
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
