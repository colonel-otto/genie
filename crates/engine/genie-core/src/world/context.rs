//! What a tick's update of one object reads and writes ([`Context`]) and its walks.

use crate::GenieError;
use crate::combat::damage::Victim;
use crate::combat::victims::KillLeft;
use crate::math::rng::{Stream, Well512};
use crate::movement::move_to::Walk;
use crate::movement::{Me, Mover, Planning};
use crate::objects::masters::Masters;
use crate::objects::release::Releaser;
use crate::unit_ai::AiMove;
use crate::unit_ai::unit::AiResume;
use crate::unit_ai::world_view::{AiStart, AiWork};
use crate::world::player::Player;
use crate::world::{BuildingDeath, World};

pub(crate) struct Context<'a> {
    /// The world the update reaches (FD-045-D28): its objects by id, every player and the
    /// world's parts. The updating object is out of its slot (its stand-in, which no lookup by
    /// id finds), so a call on another object is the world's routine at once, and a call on
    /// the updating object itself goes to this Context for after the step.
    pub(crate) w: &'a mut World,
    /// The updating object's id.
    pub(crate) me: i32,
    /// The tick's dt in milliseconds, and in seconds (`world+0x164`).
    pub(crate) dt: u32,
    pub(crate) dt_seconds: f32,
    /// The ticks so far, this one included.
    pub(crate) tick: u32,
    /// The owner's accumulated time when it has the turn: the full unit-AI update runs.
    pub(crate) turn: Option<u32>,
    pub(crate) world_time: u32,
    pub(crate) computer: bool,
    /// The owner's construction rate (attribute 195).
    pub(crate) construction_rate: f32,
    /// The unit AI's current target as its target block reads it ([`World::seen`]).
    pub(crate) ai_target: Result<Option<crate::unit_ai::Seen>, GenieError>,
    /// The objects about the target of a unit with an area effect (or about the unit, for its
    /// blast at death), which its blast may reach, read as the update starts; each hit writes
    /// them back where it ran ([`crate::combat::victims::write_back`]).
    pub(crate) blasted: Vec<Victim>,
    /// The updating object's own kill (a retaliation's), which its slot's stand-in keeps from
    /// the world until the step's end ([`crate::combat::victims::file_kill`]): the caller's
    /// after the update ([`World::kill_left`]); another object's kill reaches the world at the
    /// hit.
    pub(crate) kills: Vec<KillLeft>,
    /// What the unit's death leaves for the caller: the remains its dying sprite's end makes
    /// (`FUN_1411a4e90`: the master, the place and the dead object's facet) and the paid cost its
    /// owner takes back (attributes 98 and 164).
    pub(crate) remains: Option<(i16, [f32; 3], Option<u8>)>,
    pub(crate) refund: Vec<(i16, f32)>,
    /// What a built building's death takes from its owner before its combat part's refund: the
    /// counts of the monastery, the wonder and the castle (slot 0xf8, then slot 0xf0 with 0 for
    /// a value below 0), and its master's technology (`+0x98c`) the caller withdraws when no
    /// other building of the master stands ([`World::withdraw_tech`]).
    pub(crate) building_death: Option<BuildingDeath>,
    /// The gate's update asked its player to open or shut it ([`World::toggle_gate`], the
    /// caller's after the update).
    pub(crate) gate_toggle: bool,
    /// The unit's update stopped before its unit AI for the caller's work order
    /// ([`Unit::ai_part`]); given back, the update runs only that part.
    pub(crate) ai_resume: Option<AiResume>,
    /// The player the unit's own AI gave it to (slot 0x70, [`Unit::ai_change_owner`]) and
    /// whether it has the unit's master: the update ends with the takeover ([`Step::Captured`]).
    pub(crate) taken_over: Option<(usize, bool)>,
    /// The unit is a building whose slot 0x70 ran in the update: the whole change is the
    /// caller's ([`crate::ownership::capture::Captured::building`]).
    pub(crate) building_handover: bool,
    /// The releases (slot 0x180, [`World::release`]) and takes (slot 0x178, [`World::take`])
    /// of the updating object itself, which the view does not hold: the caller applies them
    /// after the hits' kills. The update calls every other one at once ([`crate::world::lets`]).
    pub(crate) releases: Vec<(i32, Releaser)>,
    pub(crate) takes: Vec<(i32, Releaser)>,
    /// A counted builder's leave ([`Unit::clear_own_actions`], `FUN_140aadbf0`) of the updating
    /// object itself, or of an annex of it: the building `FUN_140aaddb0` gives counts one
    /// builder less ([`World::leave_builders`]), the caller's before the releases.
    pub(crate) builder_leaves: Vec<i32>,
    /// The unit group a unit that entered a building left (`FUN_1411921d0`), with the
    /// building: the caller runs the group's removeUnit ([`World::remove_unit`]).
    pub(crate) left_group: Option<(i32, i32)>,
    /// The objects whose master leaves ghosts that died in this update ([`crate::vision::ghost`]), the
    /// caller's.
    pub(crate) ghost_deaths: Vec<i32>,
    /// The buildings whose construction started in this update: their owner watches them
    /// ([`World::watch_started`]), the caller's.
    pub(crate) started: Vec<i32>,
    /// The buildings that died in this update: their owner's watched list lets them go
    /// (`FUN_141091310`), the caller's.
    pub(crate) unwatched: Vec<i32>,
    /// The annexes a building's `set_state` (`FUN_1411dbb70`) took to its own state (slot 0xa0
    /// on each, before its own parts), the caller's after the update ([`World::set_state`]).
    pub(crate) annex_states: Vec<(i32, u8)>,
    /// The objects a builder's footprint clearing deletes (the Build's `set_state(7)`,
    /// `FUN_1415c3620` with `param_6`; their records went already), the caller's after the
    /// update ([`World::clear_object`]).
    pub(crate) cleared: Vec<crate::construction::build::Cleared>,
    /// The tracker's promotions (`FUN_14104fb20`) of the owner's units whose master changed
    /// while the unit AI ran (the unit, its old master, its new one), the caller's after the
    /// update ([`World::track_promote`]).
    pub(crate) promotions: Vec<(i32, i16, i16)>,
    /// The flares a hit on a player's object has that player make (master 112, its slot 0x1c0)
    /// at the attacker's place, doubled in their amount for a trebuchet's hit, the caller's
    /// after the update ([`World::make_flare`]).
    pub(crate) flares: Vec<(usize, [f32; 3], bool)>,
    /// The foundations another player's marks lie on, which a builder's `set_state(7)` removes
    /// when their owner is human ([`crate::construction::build::BuildStep::removes`]), the caller's after the
    /// update ([`World::remove_building`]).
    pub(crate) site_removals: Vec<i32>,
    /// The units on a foundation's footprint its builder's test met (the foundation, them):
    /// the caller sends off those the test would ([`World::evacuate`]).
    pub(crate) evacuations: Vec<(i32, Vec<i32>)>,
    /// The game's setting at `+0x680` is 1 ([`World::mode_680`]).
    pub(crate) mode_1: bool,
    /// The owner's food, wood, stone and gold ([`Player::resources`]), which a repair pays from.
    ///
    /// [`Player::resources`]: crate::world::player::Player::resources
    pub(crate) resources: [f32; 4],
    /// The owner's id and civ (`player+0x738`).
    pub(crate) player: usize,
    pub(crate) civ: u8,
    /// The owner's AI settings (`+0x608`..), which its unit AIs read.
    pub(crate) ai_settings: crate::world::player::AiSettings,
    /// The player shares its gathered resources with its allies (`+0x1426`).
    pub(crate) sharing: bool,
    /// The game's farm setting ([`crate::world::settings::Settings::walkable_farms`]).
    pub(crate) farm_stops: bool,
    /// The game's setting [`crate::world::settings::Settings::predator_hunts_end`].
    pub(crate) predator_hunts_end: bool,
    /// Every player's attribute 97 (by player), for a capture.
    pub(crate) attribute_97: &'a [f32],
    /// Every player's game status (`+0x89`), which the Artifact's owner search reads.
    pub(crate) game_status: &'a [u8],
    /// The player whose turn this tick is (`world+0x188`) and every player's computer flag.
    pub(crate) player_turn: usize,
    pub(crate) computers: &'a [bool],
    /// Each player's `chance_to_dodge_missiles` ([`crate::world::Player::dodge_chance`]).
    pub(crate) dodge_chances: &'a [u8],
    /// Each player's `chance_for_archers_to_maintain_distance`
    /// ([`crate::world::Player::keep_distance_chance`]).
    pub(crate) keep_distance_chances: &'a [u8],
    /// Some player's attribute 204 is not 0, which a building's completion reads.
    pub(crate) attribute_204: bool,
    /// [`World::mode_680`].
    pub(crate) mode_680: i32,
    /// The game's carry rule ([`crate::world::settings::Settings::carry_rule`]), which a hunt's start
    /// reads.
    pub(crate) carry_rule: bool,
    /// The game mode is one of `FUN_141302540`'s ([`scenario_mode`]).
    pub(crate) scenario: bool,
    /// The game mode's descriptor flag `+0x9c` ([`standard_mode`]) and each player's computer
    /// flag and `mAllowCapturingGaiaUnits`, which a takeover by sight reads
    /// ([`crate::vision::visibility::sight_taker`]).
    pub(crate) standard: bool,
    pub(crate) sight_takers: &'a [(bool, bool)],
    /// The attributes the owner gains from the object's update (a building it completed), which
    /// the caller adds after it.
    pub(crate) attribute_adds: Vec<(i16, f32)>,
    /// The events the owner's history records from the object's update (a castle or a wonder it
    /// completed, [`crate::construction::build::Built::history`]), which the caller adds after it.
    pub(crate) history_events: Vec<u8>,
    /// The technologies the owner researches from the object's update (a completed building's),
    /// which the caller completes after the attributes ([`World::complete_research`]).
    pub(crate) techs_done: Vec<i16>,
    /// The pastures the object's update completed, whose livestock the caller makes after the
    /// attributes ([`World::setup_pasture`]).
    pub(crate) pastures: Vec<i32>,
    /// The mule carts the object's update completed, whose search for resources the caller runs
    /// after it ([`World::cart_search`]).
    pub(crate) carts: Vec<(i32, u8)>,
    /// The buildings the object's update completed with a stack unit or that disappear when
    /// built: the caller makes the stack unit ([`World::stack_unit`]) before the technologies
    /// and destroys the building (slot 0xd0) after them.
    pub(crate) completions: Vec<(i32, Option<crate::construction::build::StackUnit>, bool)>,
    /// The work a builder found after its building (slot 0x7e8 from the Build's case 0xdf),
    /// which the caller gives after the update ([`World::order_work`]).
    pub(crate) work_order: Option<crate::orders::Work>,
    /// The Transport's unload in its state 6 (`FUN_140aa3ba0`), which the caller runs after the
    /// actions and before the unit AI part ([`World::transport_unload`]).
    pub(crate) transport_unload: bool,
    /// The unit AI's order on a target (`0x2db`) the unit took from its queue, which the caller
    /// runs after the update ([`World::order_on_target`]), and the full update's tail that reads
    /// what the order gave ([`crate::unit_ai::UnitAi::after_order`]).
    pub(crate) order_on_target: Option<(
        crate::unit_ai::QueuedOrder,
        Option<crate::unit_ai::AfterOrder>,
    )>,
    /// The unit AI's `HuntObject`s, `GatherObject`s and works (slot 0x7e8) in the order it gave
    /// them, whose unit parts the caller runs after the update in that order ([`AiWork`]): a
    /// later one's stop tears down the action an earlier one made.
    pub(crate) works: Vec<AiWork>,
    /// The foundation of the unit AI's `BuildObject`, which the caller builds after the update
    /// ([`World::build_object`]).

    /// The foundation of the unit's own build command (slot 0x380, a Gather's `set_state(6)` on
    /// a target not built yet), which the caller runs after the update ([`World::build_command`]).
    pub(crate) build_command: Option<i32>,
    /// A Gather's walk back to reseed ended (state 0xf2, whether it arrived), which the caller
    /// reseeds after the update ([`World::reseed_after_walk`]).
    pub(crate) reseed: Option<bool>,
    /// The unit AI's moves off foundations' footprints (slot 0x148), in order, which the caller
    /// runs after the update ([`World::move_aside`]).
    pub(crate) move_aside: Vec<AiMove>,
    /// The transports an Enter's `set_state(4)` sent to where they meet the unit (slot 0x340),
    /// which the caller moves after the update ([`World::move_transport`]'s move).
    pub(crate) transport_moves: Vec<(i32, [f32; 3])>,
    /// The actions the unit's slots 0x388 and 0x378 gave while its unit AI ran ([`AiStart`]),
    /// which the caller makes the unit's after the update, in order.
    pub(crate) starts: Vec<AiStart>,
    /// The work a builder gave an annex whose main building takes it (the foundation, the
    /// seconds; [`crate::construction::build::Worked::Main`]), which the caller runs after the update
    /// ([`World::work_building`]).
    pub(crate) main_works: Vec<(i32, f32)>,
    /// The livestock a hunt sends to a drop site with its herd, which the caller sends after the
    /// update ([`World::lead_herd`]).
    pub(crate) leads: Option<(i32, i32)>,
}

/// What an object's step gathered in its Context for the world to apply after it
/// ([`crate::world::World::step_object`]): the Context's fields of the same meaning; the left
/// group carries the unit's id.
pub(crate) struct ObjectOutputs {
    /// [`Context`]'s `attribute_adds`.
    pub(in crate::world) adds: Vec<(i16, f32)>,
    /// [`Context`]'s `order_on_target`.
    pub(in crate::world) ordered: Option<(
        crate::unit_ai::QueuedOrder,
        Option<crate::unit_ai::AfterOrder>,
    )>,
    /// [`Context`]'s `history_events`.
    pub(in crate::world) history_events: Vec<u8>,
    /// [`Context`]'s `works`.
    pub(in crate::world) works: Vec<AiWork>,
    /// [`Context`]'s `leads`.
    pub(in crate::world) leads: Option<(i32, i32)>,
    /// [`Context`]'s `build_command`.
    pub(in crate::world) build_command: Option<i32>,
    /// [`Context`]'s `reseed`.
    pub(in crate::world) reseed: Option<bool>,
    /// [`Context`]'s `move_aside`.
    pub(in crate::world) move_aside: Vec<AiMove>,
    /// [`Context`]'s `transport_moves`.
    pub(in crate::world) transport_moves: Vec<(i32, [f32; 3])>,
    /// [`Context`]'s `starts`.
    pub(in crate::world) starts: Vec<AiStart>,
    /// [`Context`]'s `techs_done`.
    pub(in crate::world) techs_done: Vec<i16>,
    /// [`Context`]'s `pastures`.
    pub(in crate::world) pastures: Vec<i32>,
    /// [`Context`]'s `carts`.
    pub(in crate::world) carts: Vec<(i32, u8)>,
    /// [`Context`]'s `completions`.
    pub(in crate::world) completions:
        Vec<(i32, Option<crate::construction::build::StackUnit>, bool)>,
    /// [`Context`]'s `work_order`.
    pub(in crate::world) work_order: Option<crate::orders::Work>,
    /// [`Context`]'s `transport_unload`.
    pub(in crate::world) transport_unload: bool,
    /// [`Context`]'s `kills`.
    pub(in crate::world) kills: Vec<KillLeft>,
    /// [`Context`]'s `remains`.
    pub(in crate::world) remains: Option<(i16, [f32; 3], Option<u8>)>,
    /// [`Context`]'s `refund`.
    pub(in crate::world) refund: Vec<(i16, f32)>,
    /// [`Context`]'s `annex_states`.
    pub(in crate::world) annex_states: Vec<(i32, u8)>,
    /// [`Context`]'s `cleared`.
    pub(in crate::world) cleared: Vec<crate::construction::build::Cleared>,
    /// [`Context`]'s `site_removals`.
    pub(in crate::world) site_removals: Vec<i32>,
    /// [`Context`]'s `flares`.
    pub(in crate::world) flares: Vec<(usize, [f32; 3], bool)>,
    /// [`Context`]'s `promotions`.
    pub(in crate::world) promotions: Vec<(i32, i16, i16)>,
    /// [`Context`]'s `main_works`.
    pub(in crate::world) main_works: Vec<(i32, f32)>,
    /// [`Context`]'s `building_death`.
    pub(in crate::world) building_death: Option<BuildingDeath>,
    /// [`Context`]'s `gate_toggle`.
    pub(in crate::world) gate_toggle: bool,
    /// [`Context`]'s `ai_resume`.
    pub(in crate::world) ai_resume: Option<AiResume>,
    /// [`Context`]'s `releases`.
    pub(in crate::world) releases: Vec<(i32, Releaser)>,
    /// [`Context`]'s `builder_leaves`.
    pub(in crate::world) builder_leaves: Vec<i32>,
    /// [`Context`]'s `left_group`.
    pub(in crate::world) left_group: Option<(i32, i32, i32)>,
    /// [`Context`]'s `takes`.
    pub(in crate::world) takes: Vec<(i32, Releaser)>,
    /// [`Context`]'s `ghost_deaths`.
    pub(in crate::world) ghost_deaths: Vec<i32>,
    /// [`Context`]'s `started`.
    pub(in crate::world) started: Vec<i32>,
    /// [`Context`]'s `unwatched`.
    pub(in crate::world) unwatched: Vec<i32>,
    /// [`Context`]'s `evacuations`.
    pub(in crate::world) evacuations: Vec<(i32, Vec<i32>)>,
}

impl Context<'_> {
    /// What the step gathered ([`ObjectOutputs`]), taken out of the Context; `id` is the
    /// unit's.
    pub(in crate::world) fn outputs(&mut self, id: i32) -> ObjectOutputs {
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
            starts: std::mem::take(&mut self.starts),
            techs_done: std::mem::take(&mut self.techs_done),
            pastures: std::mem::take(&mut self.pastures),
            carts: std::mem::take(&mut self.carts),
            completions: std::mem::take(&mut self.completions),
            work_order: self.work_order.take(),
            transport_unload: std::mem::take(&mut self.transport_unload),
            kills: std::mem::take(&mut self.kills),
            remains: self.remains.take(),
            refund: std::mem::take(&mut self.refund),
            annex_states: std::mem::take(&mut self.annex_states),
            cleared: std::mem::take(&mut self.cleared),
            site_removals: std::mem::take(&mut self.site_removals),
            flares: std::mem::take(&mut self.flares),
            promotions: std::mem::take(&mut self.promotions),
            main_works: std::mem::take(&mut self.main_works),
            building_death: self.building_death.take(),
            gate_toggle: std::mem::take(&mut self.gate_toggle),
            ai_resume: self.ai_resume.take(),
            releases: std::mem::take(&mut self.releases),
            builder_leaves: std::mem::take(&mut self.builder_leaves),
            left_group: self.left_group.take().map(|(g, b)| (g, id, b)),
            takes: std::mem::take(&mut self.takes),
            ghost_deaths: std::mem::take(&mut self.ghost_deaths),
            started: std::mem::take(&mut self.started),
            unwatched: std::mem::take(&mut self.unwatched),
            evacuations: std::mem::take(&mut self.evacuations),
        }
    }
}

/// What a walk takes of the update beside the world: the dt in seconds, the world's time, and
/// the owner's computer flag and construction rate.
#[derive(Clone, Copy)]
pub(crate) struct Pace {
    pub(crate) dt: f32,
    pub(crate) now: u32,
    pub(crate) computer: bool,
    pub(crate) construction_rate: f32,
}

/// The world's parts beside a walk ([`World::walk_beside`]), which an update reads and changes
/// while its unit walks.
pub(crate) struct Beside<'b> {
    pub(crate) players: &'b mut Vec<Player>,
    pub(crate) masters: &'b [Masters],
}

impl World {
    /// A walk at `pace` ([`Context::walk`]) and the world's parts beside it, each its own
    /// borrow.
    pub(crate) fn walk_beside<'b>(
        &'b mut self,
        pace: Pace,
        mover: Option<&'b mut Mover>,
        me: Me,
    ) -> (Walk<'b>, Beside<'b>) {
        let Self {
            players,
            masters,
            spatial,
            pathing,
            groups,
            streams,
            ..
        } = self;
        let walk = Walk {
            mover,
            me,
            dt: pace.dt,
            world: spatial,
            plan: Some(Planning {
                pathing,
                now: pace.now,
                computer: pace.computer,
                object: streams.get(Stream::Object),
                construction_rate: pace.construction_rate,
                groups,
            }),
        };
        let beside = Beside { players, masters };
        (walk, beside)
    }
}

impl Context<'_> {
    /// What a walk takes of the update beside the world ([`World::walk_beside`]).
    pub(crate) const fn pace(&self) -> Pace {
        Pace {
            dt: self.dt_seconds,
            now: self.world_time,
            computer: self.computer,
            construction_rate: self.construction_rate,
        }
    }
}

impl Context<'_> {
    /// A walk and the Sprite stream, for an attacker's update.
    pub(crate) fn walk_and_sprites<'b>(
        &'b mut self,
        mover: Option<&'b mut Mover>,
        me: Me,
    ) -> (Walk<'b>, &'b mut Well512) {
        let (object, sprite) = self.w.streams.two(Stream::Object, Stream::Sprite);
        (
            Walk {
                mover,
                me,
                dt: self.dt_seconds,
                world: &mut self.w.spatial,
                plan: Some(Planning {
                    pathing: &mut self.w.pathing,
                    now: self.world_time,
                    computer: self.computer,
                    object,
                    construction_rate: self.construction_rate,
                    groups: &self.w.groups,
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
            self.w
                .streams
                .three(Stream::Object, Stream::Sprite, Stream::Action);
        (
            Walk {
                mover,
                me,
                dt: self.dt_seconds,
                world: &mut self.w.spatial,
                plan: Some(Planning {
                    pathing: &mut self.w.pathing,
                    now: self.world_time,
                    computer: self.computer,
                    object,
                    construction_rate: self.construction_rate,
                    groups: &self.w.groups,
                }),
            },
            sprite,
            action,
        )
    }

    /// A walk and another stream than the walk's Object stream.
    pub(crate) fn walk_with<'b>(
        &'b mut self,
        mover: Option<&'b mut Mover>,
        me: Me,
        other: Stream,
    ) -> (Walk<'b>, &'b mut Well512) {
        let (object, other) = self.w.streams.two(Stream::Object, other);
        (
            Walk {
                mover,
                me,
                dt: self.dt_seconds,
                world: &mut self.w.spatial,
                plan: Some(Planning {
                    pathing: &mut self.w.pathing,
                    now: self.world_time,
                    computer: self.computer,
                    object,
                    construction_rate: self.construction_rate,
                    groups: &self.w.groups,
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
            world: &mut self.w.spatial,
            plan: Some(Planning {
                pathing: &mut self.w.pathing,
                now: self.world_time,
                computer: self.computer,
                object: self.w.streams.get(Stream::Object),
                construction_rate: self.construction_rate,
                groups: &self.w.groups,
            }),
        }
    }
}
