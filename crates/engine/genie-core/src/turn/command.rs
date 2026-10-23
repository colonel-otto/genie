//! The commands ([`Command`]) and their dispatch ([`Turns::run`]): the handlers of the
//! players' orders, the buildings' placement and the production queue's.

use crate::build::{BUILD, BuildAction, BuildTask, Construction};
use crate::lifecycle::{Class, Lifecycle};
use crate::masters::{Master, Masters};
use crate::movement::{Mover, footprint_gap};
use crate::order::{Interact, MoveOrder};
use crate::production::Production;
use crate::rng::Stream;
use crate::spatial::Placed;
use crate::tasks::{Actor, TaskTarget};
use crate::turn::unit::{POPULATION, Unit};
use crate::turn::view::Wait;
use crate::turn::world::{World, clear_look_of, count_class, task_search, task_target_with};
use crate::turn::{
    BLOCKS_RESEARCH, BUILDING_COST, BUILDING_KIND, COMPUTER_QUEUE_LIMIT, FARM, GATE, Gate,
    ONE_UNIT_GAME, PASTURE_1889, QUEUE_LIMIT, RESEARCH_ACTION, RICE_FARM, STATIC_TYPES_END, Turns,
    VILLAGER, VILLAGER_ANY, villager,
};
use crate::unit_ai::{BUILD_ACTION, BUILD_ORDER, Notify, UnitAi, Unsupported};
use crate::visibility::{BuildingSight, Seer, capture_kind};

/// A building's gather point (`+0xc30`..`+0xc44`: it exists, x, y, z, the target and the target's
/// master; the save's `gather_point_*`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GatherPoint {
    /// The point its trained units walk to.
    pub point: [f32; 3],
    /// The object it is on, −1 for none.
    pub target: i32,
    /// The target's master, −1 without a target.
    pub target_master: i16,
}

/// A recorded command, run in the command phase of its tick: after the frame seed's `SeedAll`, before
/// the players update (`TRIBE_Command::do_command`, `0x140e77b00`). The slice runs only what other
/// players' lanes see of a command; the issuing player's lane stops.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    /// BUILD (0x66, `FUN_140e7a400`): the player places a foundation of `master` at `position`
    /// (state 0, hit points 1), with a unit AI of the given type if its class gets one; `Err` when
    /// the slice cannot tell. The player pays the master's cost, and each builder, in packet order,
    /// takes the master it builds with and starts a Build action (`Turns::masters`). The slice
    /// takes the placement as given: the recording's game accepted it.
    Build {
        /// The player who places it.
        player: usize,
        /// The foundation's master.
        master: i16,
        /// The unit AI type its class gets, if any, or why the slice cannot tell.
        unit_ai: Result<Option<u32>, Unsupported>,
        /// Where it is placed.
        position: [f32; 3],
        /// The units told to build it, in packet order.
        builders: Vec<i32>,
        /// `+0x1e`: queued (a builder with a unit AI takes it as an order in its queue).
        queued: bool,
        /// `+0x1f` is 0: a builder with a unit AI holding an order not ready takes it as an order
        /// too, and the queue's orders become ready after (`FUN_140c42b60`).
        commits: bool,
    },
    /// `BUILD_WALL` (0x69, `FUN_140e7b2e0`): a line of the player's wall foundations and its
    /// builders shared out over them ([`crate::wall`]).
    BuildWall(crate::wall::Wall),
    /// SET GATHER POINT (0x78, `FUN_140e7e9a0`): each of the player's objects of `ids` that takes
    /// a gather point (its master garrisons, `+0x5d4`; the list of units it trains,
    /// `FUN_141625a20`, is not modelled) takes `point` and `target` with its master, or with
    /// `own` itself (`FUN_1411c7ff0`); a target of −1 at (−1, −1) clears it. The ids of other
    /// players' objects are left out (the game setting `+0x1074` that lets them is not modelled),
    /// and the unit AI's command queue (`+0x1f0`, which its slot 0x210 empties) is not held.
    GatherPoint {
        /// The player whose objects take it.
        player: usize,
        /// The objects of the packet.
        ids: Vec<i32>,
        /// The point.
        point: [f32; 2],
        /// The object the point is on, −1 for none.
        target: i32,
        /// That object's master.
        target_master: i16,
        /// The building itself is the gather point (`+0x18`).
        own: bool,
    },
    /// QUEUE (0x81, the player's queue `FUN_141614450`, the building's `FUN_1411cbc40`): `amount`
    /// units of master `unit` at one building with the train button `button`
    /// ([`Turns::queue`]).
    Queue {
        /// The player who queues.
        player: usize,
        /// The buildings the packet lists (`+0x14..`, its `num` of them).
        buildings: Vec<i32>,
        /// The master to train (0x712 for the villager).
        unit: i16,
        /// How many.
        amount: u16,
        /// `button_location_id`: the train button, −1 for none.
        button: i32,
    },
    /// RESEARCH (0x65, `0x140e7e270`): the technology `tech` at the building `building`
    /// ([`Turns::research`]). `count` (`+0x8`) counts the buildings the packet lists, `target`
    /// (`+0xc`) is −1 for a research to run, and `shift` (`+0x10`) asks for five at once.
    Research {
        /// The player who researches.
        player: usize,
        /// The building of the packet.
        building: i32,
        /// `+0x8`: how many buildings the packet lists.
        count: i16,
        /// `+0x11..`: the buildings the packet lists.
        buildings: Vec<i32>,
        /// `+0xa`: the technology.
        tech: i16,
        /// `+0xc`: −1 for a research to run.
        target: i32,
        /// `+0x10`: five at once.
        shift: bool,
    },
    /// A command the slice runs part of or none, for a reason the decoder names: it stops the
    /// issuing player's lane.
    Unmodelled {
        /// The issuing player.
        player: usize,
        /// Why its command is not modelled.
        why: Unsupported,
    },
    /// GAME (0x67, `0x140e7c330`) sub-commands 0x10 and 0x13: the player's `autoFarmQueue`
    /// (`+0x1424`) or `autoFishTrapQueue` (`+0x1425`) flips (`fishtrap` true for 0x13). The other
    /// sub-commands are the decoder's [`Command::Unmodelled`].
    AutoQueue {
        /// The player.
        player: usize,
        /// The fish trap queue flips (0x13), else the farm queue (0x10).
        fishtrap: bool,
    },
    /// GAME (0x67, `0x140e7c330`) sub-command 0x14 [`default_stance`]: the player's offensive
    /// posture (`+0x71c`, `WorldPlayer` slot 0x14 `FUN_14163a630`) flips between 0 and 1.
    Posture {
        /// The player.
        player: usize,
    },
    /// GAME (0x67, `0x140e7c330`) sub-command 6 (`FUN_1415c92c0`): a cheat code of the player at
    /// `+8`. Without the game's cheats (`mCheatsEnabled`,
    /// [`crate::settings::Settings::cheats_enabled`]) it does nothing; with them the world's
    /// `+0x818` sets and the cheat runs (`FUN_1415c62c0`), which is not modelled. (`do_command`
    /// lets a player's cheat for another player through only with cheats or `+0xb12`.)
    Cheat {
        /// The player at `+8`.
        player: usize,
        /// The cheat's code (`+0xa`).
        code: i16,
    },
    /// `BUILDING_COMMAND_QUEUE` (0x2d, `0x140e64310`): the objects `ids` (at most a human
    /// player's 60) queue a step to `target` (−1 for none) or `point` ([`Turns::command_queue`]).
    CommandQueue {
        /// The player.
        player: usize,
        /// The objects of the packet.
        ids: Vec<i32>,
        /// The object the step is on, −1 for none.
        target: i32,
        /// Its point.
        point: [f32; 2],
    },
    /// `GO_BACK_TO_WORK` (0x80 with one object, `+4`; 0x86 with a list, its count at `+4`, the ids
    /// from `+8`): the player's buildings let their villagers back out to work
    /// ([`Turns::go_back_to_work`]).
    GoBackToWork {
        /// The player.
        player: usize,
        /// Its buildings.
        ids: Vec<i32>,
    },
    /// `TOWN_BELL` (0x7f, `do_command` case 0x7f): the town centre `id` rings its owner's bell
    /// (`activate`) or ends it ([`Turns::town_bell`]).
    TownBell {
        /// The player.
        player: usize,
        /// The town centre.
        id: i32,
        /// Ring (1) or end (0).
        activate: bool,
    },
    /// `CANCEL_BUILD` (0x6a, `do_command` case 0x6a): the player deletes its object `id`
    /// ([`Turns::delete`]).
    Delete {
        /// The player.
        player: usize,
        /// The object.
        id: i32,
    },
    /// FLARE (0x73, `0x140e7bfd0`): every player from 1 below `players` (`+0x10`, at most the
    /// world's) gets a flare at `target`'s place (`+4`, −1 for none) or at `point` (`+8`, `+0xc`)
    /// ([`Turns::flare`]).
    Flare {
        /// The player who sent it.
        player: usize,
        /// The object it is on, −1 for none.
        target: i32,
        /// Its point without a target.
        point: [f32; 2],
        /// The players the command counts (`+0x10`).
        players: u8,
    },
    /// GIVE ATTRIBUTES (0xc4, `FUN_140e7d040`): the player's tribute to player `to`
    /// ([`Turns::tribute`]).
    Tribute {
        /// The player who gives.
        player: usize,
        /// The player who receives (`+0x2c`).
        to: usize,
        /// The four entries.
        gifts: [crate::tribute::Gift; 4],
    },
    /// GATE (0x72, `do_command` case 0x72): the player's building `id` toggles its gate lock
    /// ([`Turns::gate`]).
    Gate {
        /// The player.
        player: usize,
        /// Its building.
        id: i32,
    },
    /// UNLOAD (0x6f, `FUN_14161f410`): the units `ids` (a building, the units inside it; a unit
    /// inside one, itself) let units out ([`Turns::unload`]) toward `point` (−1, −1 for none),
    /// those `flag` and `unit_type` pick (`FUN_14118a610`); `sound` (`+0x18`) plays the
    /// building's.
    Unload {
        /// The player.
        player: usize,
        /// Its objects.
        ids: Vec<i32>,
        /// Where they go.
        point: [f32; 2],
        /// Which units leave (`FUN_14118a610`'s switch).
        flag: u8,
        /// The unit type or object the flag reads.
        unit_type: i32,
        /// `+0x18`.
        sound: bool,
    },
    /// `TRANSFORM_OBJECT` (0x29, `RGE_Command::do_command`'s case): the player's object `id`
    /// takes the master `master` ([`Turns::transform_object`]).
    TransformObject {
        /// The player.
        player: usize,
        /// The object (`+8`).
        id: i32,
        /// The new master (`+4`).
        master: i16,
    },
    /// RESIGN (0x0b, `FUN_140d25af0`): the comm player `player` resigns, or drops (`dropped`)
    /// ([`Turns::resign`]).
    Resign {
        /// The comm player (the world player of the same number in the slice's games).
        player: usize,
        /// A drop from the game, not a resignation.
        dropped: bool,
    },
    /// `AUTO_SCOUT` (0x26, `FUN_140e666f0`): each of `ids` (at most 60, the decoder's) takes the
    /// auto scout order, or the stop order when it scouts already ([`Turns::auto_scout`]).
    AutoScout {
        /// The player.
        player: usize,
        /// Its units.
        ids: Vec<i32>,
    },
    /// STOP (0x01, `FUN_14161e200`): the units `ids` (at most 60, the decoder's) stop
    /// ([`Turns::stop_command`]).
    Stop {
        /// The player whose units stop.
        player: usize,
        /// The units.
        ids: Vec<i32>,
    },
    /// PATROL (0x15, `FUN_140e69cc0`): the units `ids` (the packet's count at `+0x4`, the ids
    /// from `+0x5c`) patrol through `waypoints` (1 to 10, their count at `+0xb`, x from `+0xc`, y
    /// from `+0x34`); `reset` (`+0x8`) drops their unit AIs' saved patrols ([`Turns::patrol`]).
    Patrol {
        /// The player whose units patrol.
        player: usize,
        /// The units.
        ids: Vec<i32>,
        /// The waypoints.
        waypoints: Vec<[f32; 2]>,
        /// `+0x8`.
        reset: bool,
    },
    /// `SEEK_SHELTER` (0x2c, `FUN_140e6a3a0`): the villagers of `ids` (at most 60, the decoder's)
    /// seek shelter in the player's buildings ([`Turns::seek_shelter`]).
    SeekShelter {
        /// The player whose units seek shelter.
        player: usize,
        /// The units.
        ids: Vec<i32>,
    },
    /// `UNIT_AI_STATE` (0x12, `FUN_140e6a7c0`): the unit AIs of `ids` (at most 60, the decoder's)
    /// take the stance `state` ([`Turns::unit_ai_state`]).
    UnitAiState {
        /// The player whose units take it.
        player: usize,
        /// The units.
        ids: Vec<i32>,
        /// The stance.
        state: u8,
    },
    /// `UNIT_ORDER` (0x75, `FUN_140e7f200` -> `FUN_14161e370`): the units `ids` (at most 60, the
    /// decoder's) take the order `action` ([`Turns::unit_order`]) with the object `target`
    /// (`tId`, `+8`, which action 5 reads) and the flags `+0x1d`, `+0x1e`; `human` (`+0x20`) sets
    /// their last human order time first. Its point, `qId` and `param` are not read by the
    /// actions modelled.
    UnitOrder {
        /// The player whose units take it.
        player: usize,
        /// The units.
        ids: Vec<i32>,
        /// `tId`: the target object's id.
        target: i32,
        /// `+0x1d`: a garrison goes through the units' unit AIs (their slot 0x30); a cancel
        /// (action 4) cancels up to five of a kind.
        through_ai: bool,
        /// `+0x1e`: a garrison takes the buildings of the target's master near it.
        nearby: bool,
        /// The order's action (4: a queue's cancel, 8: the drop-off).
        action: u8,
        /// `+0x18`: a cancel's unit index in the queue.
        param: u32,
        /// `+0x20`: it sets their last human order time.
        human: bool,
    },
    /// INTERACT (0x00), MOVE (0x03) or RETREAT (0x23): before the order runs, each listed unit's AI
    /// resets its idle timeout ([`UnitAi::reset_for_order`]), in list order, wherever the unit is.
    /// INTERACT's order then runs ([`Interact`]), and MOVE's ([`MoveOrder`]); RETREAT is not
    /// modelled.
    Order {
        /// The player who orders.
        player: usize,
        /// The units of the packet.
        units: Vec<i32>,
        /// The order they take, None for a command that only resets their idle timeouts.
        order: Option<Order>,
    },
}

/// What an order command gives its units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Order {
    /// INTERACT: work on, attack or follow a target.
    Interact(Interact),
    /// MOVE: walk to a point.
    Move(MoveOrder),
    /// FOLLOW's target (`+0x8`).
    Follow(i32),
}

/// RESEARCH's fields ([`Command::Research`] but the player).
pub(in crate::turn) struct ResearchAt {
    pub(in crate::turn) building: i32,
    pub(in crate::turn) count: i16,
    pub(in crate::turn) buildings: Vec<i32>,
    pub(in crate::turn) tech: i16,
    pub(in crate::turn) target: i32,
    pub(in crate::turn) shift: bool,
}

/// QUEUE's fields ([`Command::Queue`] but the player).
pub(in crate::turn) struct QueueAt {
    pub(in crate::turn) buildings: Vec<i32>,
    pub(in crate::turn) unit: i16,
    pub(in crate::turn) amount: u16,
    pub(in crate::turn) button: i32,
}

/// The player's attribute game mode 16's QUEUE reads (6, `+0x30`), and its bound
/// (`DAT_14330eb8c`).
const CURRENT_AGE: i16 = 6;
const ONE_UNIT_AGE: f32 = crate::image::turn::ONE_UNIT_AGE;

impl Turns {
    /// Run a command in the command phase. Where the slice does not model what the command does to
    /// the issuing player's units, that player's lane stops.
    pub(in crate::turn) fn run(&mut self, command: &Command) {
        match *command {
            Command::Build {
                player,
                master,
                unit_ai,
                position,
                ref builders,
                queued,
                commits,
            } => {
                let master = self.farm_on_rice_terrain(master, position);
                // the pending debit first (FUN_141630d10 with 0), then the builders' test, the
                // placement test and the payment's: a refused BUILD places nothing
                if let Err(e) = self.debit_listed(player, master) {
                    self.stop_player(player, e);
                    return;
                }
                match self.build_has_builder(player, master, builders) {
                    Ok(true) => {}
                    Ok(false) => return,
                    Err(e) => {
                        self.stop_player(player, e);
                        return;
                    }
                }
                match self.build_allowed(player, master, [position[0], position[1]]) {
                    Ok(true) => {}
                    Ok(false) => return,
                    Err(e) => {
                        self.stop_player(player, e);
                        return;
                    }
                }
                let built =
                    self.found(player, master, unit_ai, position)
                        .and_then(|(id, position)| {
                            self.task_builders(
                                player,
                                id,
                                master,
                                position,
                                builders,
                                (queued, commits),
                            )
                        });
                if let Err(e) = built {
                    self.stop_player(player, e);
                }
            }
            Command::Queue {
                player,
                ref buildings,
                unit,
                amount,
                button,
            } => {
                let queue = QueueAt {
                    buildings: buildings.clone(),
                    unit,
                    amount,
                    button,
                };
                if let Err(e) = self.queue(player, &queue) {
                    self.stop_player(player, e);
                }
            }
            Command::Research {
                player,
                building,
                count,
                ref buildings,
                tech,
                target,
                shift,
            } => {
                let r = ResearchAt {
                    building,
                    count,
                    buildings: buildings.clone(),
                    tech,
                    target,
                    shift,
                };
                if let Err(e) = self.research(player, &r) {
                    self.stop_player(player, e);
                }
            }
            Command::Unmodelled { player, why } => {
                self.stop_player(player, why);
            }
            Command::AutoQueue { player, fishtrap } => {
                let owner = &mut self.players[player];
                if fishtrap {
                    owner.auto_fishtrap_queue = !owner.auto_fishtrap_queue;
                } else {
                    owner.auto_farm_queue = !owner.auto_farm_queue;
                }
            }
            Command::Posture { player } => {
                let owner = &mut self.players[player];
                owner.posture = i32::from(owner.posture == 0);
            }
            Command::Cheat { player, code: _ } => {
                if self.settings.cheats_enabled {
                    self.stop_player(player, Unsupported("a cheat code (FUN_1415c62c0)"));
                }
            }
            Command::Gate { player, id } => {
                if let Err(e) = self.gate(player, id) {
                    self.stop_player(player, e);
                }
            }
            Command::CommandQueue {
                player,
                ref ids,
                target,
                point,
            } => {
                if let Err(e) = self.command_queue(player, ids, target, point) {
                    self.stop_player(player, e);
                }
            }
            Command::BuildWall(ref w) => {
                if let Err(e) = self.build_wall(w) {
                    self.stop_player(w.player, e);
                }
            }
            Command::Delete { player, id } => {
                if let Err(e) = self.delete(player, id) {
                    self.stop_player(player, e);
                }
            }
            Command::GoBackToWork { player, ref ids } => {
                if let Err(e) = self.go_back_to_work(player, ids) {
                    self.stop_player(player, e);
                }
            }
            Command::TownBell {
                player,
                id,
                activate,
            } => {
                if let Err(e) = self.town_bell(player, id, activate) {
                    self.stop_player(player, e);
                }
            }
            Command::Flare {
                player,
                target,
                point,
                players,
            } => {
                if let Err(e) = self.flare(target, point, players) {
                    self.stop_player(player, e);
                }
            }
            Command::Tribute {
                player,
                to,
                ref gifts,
            } => {
                if let Err(e) = self.tribute(player, to, gifts) {
                    self.stop_player(player, e);
                }
            }
            Command::Unload {
                player,
                ref ids,
                point,
                flag,
                unit_type,
                sound,
            } => {
                if let Err(e) = self.unload(player, ids, point, flag, unit_type, sound) {
                    self.stop_player(player, e);
                }
            }
            Command::UnitOrder {
                player,
                ref ids,
                target,
                through_ai,
                nearby,
                action,
                param,
                human,
            } => {
                if human {
                    self.human_order(ids);
                }
                let result = match action {
                    5 => self.garrison_order(player, ids, target, through_ai, nearby),
                    4 => self.cancel_order(ids, param, through_ai),
                    _ => self.unit_order(player, ids, action),
                };
                if let Err(e) = result {
                    self.stop_player(player, e);
                }
            }
            Command::TransformObject { player, id, master } => {
                if let Err(e) = self.transform_object(player, id, master) {
                    self.stop_player(player, e);
                }
            }
            Command::Resign { player, dropped } => {
                if let Err(e) = self.resign(player, dropped) {
                    self.stop_player(player, e);
                }
            }
            Command::AutoScout { player, ref ids } => {
                if let Err(e) = self.auto_scout(player, ids) {
                    self.stop_player(player, e);
                }
            }
            Command::Stop { player, ref ids } => {
                if let Err(e) = self.stop_command(player, ids) {
                    self.stop_player(player, e);
                }
            }
            Command::Patrol {
                player,
                ref ids,
                ref waypoints,
                reset,
            } => {
                if let Err(e) = self.patrol(player, ids, waypoints, reset) {
                    self.stop_player(player, e);
                }
            }
            Command::SeekShelter { player, ref ids } => {
                if let Err(e) = self.seek_shelter(player, ids) {
                    self.stop_player(player, e);
                }
            }
            Command::UnitAiState {
                player,
                ref ids,
                state,
            } => {
                if let Err(e) = self.unit_ai_state(player, ids, state) {
                    self.stop_player(player, e);
                }
            }
            Command::GatherPoint {
                player,
                ref ids,
                point,
                target,
                target_master,
                own,
            } => {
                if let Err(e) = self.gather_point(player, ids, point, (target, target_master), own)
                {
                    self.stop_player(player, e);
                }
            }
            Command::Order {
                player,
                ref units,
                order,
            } => {
                // the command's check (MOVE's FUN_140e553f0, INTERACT's FUN_140e55250): a unit the
                // world does not hold, or of another player without the setting +0x1074, becomes
                // -1; a human player's command left without a unit does nothing
                let checked: Vec<i32>;
                let units: &[i32] = if matches!(order, Some(Order::Move(_) | Order::Interact(_))) {
                    checked = self.checked_units(player, units);
                    if checked.iter().all(|&id| id == -1) && !self.players[player].computer {
                        return;
                    }
                    &checked
                } else {
                    units
                };
                // FOLLOW (FUN_140e66a60) of a target the world lacks does nothing
                if let Some(Order::Follow(target)) = order
                    && self.object(target).is_none()
                {
                    return;
                }
                // +0x15: each unit with a unit AI takes the world time as its last human order
                // time (FUN_140e5e780), after the resets; FOLLOW sets it when a unit the world
                // holds belongs to a player who is not a computer (FUN_141659cf0)
                let human = match order {
                    Some(Order::Interact(o)) => o.human,
                    Some(Order::Move(o)) => o.human,
                    Some(Order::Follow(_)) => units.iter().any(|&id| {
                        self.object(id)
                            .is_some_and(|(p, _)| !self.players[p].computer)
                    }),
                    None => false,
                };
                self.reset_order_units(units);
                if human {
                    self.human_order(units);
                }
                let result = match order {
                    Some(Order::Interact(o)) => self.interact(player, units, &o),
                    Some(Order::Move(o)) => self.move_order(player, units, &o),
                    Some(Order::Follow(target)) => self.follow(player, units, target),
                    None => Err(Unsupported("an order moves its units")),
                };
                if let Err(e) = result {
                    self.stop_player(player, e);
                }
            }
        }
    }

    /// `FUN_140e5e780`: each unit of `ids` with a unit AI takes the world time as its last human
    /// order time (`+0x650`).
    pub(in crate::turn) fn human_order(&mut self, ids: &[i32]) {
        for &id in ids {
            let unit = self
                .players
                .iter_mut()
                .flat_map(|p| p.objects.iter_mut())
                .find(|o| o.id == id)
                .filter(|o| o.ai.is_some());
            if let Some(u) = unit {
                u.last_human_order = self.world_time;
                self.spatial.set_gait(u.id, u.gait());
            }
        }
    }

    /// The new foundation of BUILD and of `BUILD_WALL`'s tiles (the master's slot 0x108,
    /// [`Turns::make_building`]), then the player pays the master's cost (each entry flagged
    /// paid). Its id and position.
    pub(crate) fn found(
        &mut self,
        player: usize,
        master: i16,
        unit_ai: Result<Option<u32>, Unsupported>,
        position: [f32; 3],
    ) -> Result<(i32, [f32; 3]), Unsupported> {
        let made = self.make_building(player, master, unit_ai, position)?;
        let cost = self.master_of(player, master)?.cost.clone();
        let owner = &mut self.players[player];
        for &(resource, amount, paid) in &cost {
            let slot = usize::try_from(resource)
                .ok()
                .and_then(|r| owner.resources.get_mut(r))
                .ok_or(Unsupported("a cost in a resource other than the four"))?;
            if paid {
                *slot -= f32::from(amount);
            }
        }
        Ok(made)
    }

    /// A new building (the master's slot 0x40 or 0x108, `FUN_1411ac5c0` / `FUN_1411aca00` with
    /// their init: the static, animated, moving and combat setups, then the constructor
    /// `FUN_1411de5f0`) of `master` for `player` at `position`, the ground's height: its id
    /// (`mNextObjectId`), its unit AI of the given type if its class gets one, the combat setup's
    /// class counters ([`count_class`]), its powerup offset (the Object stream's "setup" draw)
    /// and a building with a train location its paid cost (attribute 247); the static init's
    /// storage; its place and sight (its place links its tiles to its anchor, `FUN_1411e6780`).
    /// The constructor: masters 50 and 1889 on snow keep their snow (`+0xc61` 2); a master
    /// neither of classes 51, 54 nor 357, 1188, nor an annex (`+0x9b8` −1) takes its build time
    /// as a foundation (a build time above 0: hit points 1, a farm's terrain,
    /// [`Turns::construction_terrain`]; build points 0), the others are built already (build
    /// points their train time, their master's hit points); one with build points 0 and a
    /// build time above 0, not of classes 51 or 54, turns desolid (`FUN_1411cf770`: no record,
    /// its tiles unlinked); its footprint is cleared ([`Turns::clear_footprint`]); each annex
    /// slot (`+0x990`) of a master the player has makes its annex the same way at the slot's
    /// offset (`+0x998`, `+0x9a8`), depth first, linked to the building (`+0xbb8`), out of its
    /// owner's watched list unless the building is a gate (39), desolid when its master is tall,
    /// its own footprint cleared; `+0xcd0` notes mangrove or rice terrain under it
    /// (`FUN_141188db0`); last its owner watches it (`FUN_1411cc980`, [`Turns::watch_started`]).
    /// Not held: the construction sprite, the facets, the world's `+0x154`, slot 0x840's
    /// `+0xba0` and `+0xbf8`, and the wonder's `FUN_1411665c0`.
    ///
    /// # Errors
    ///
    /// A master the player has not got or without its building part, an annex of another type,
    /// and what the clearing and the attributes do not model.
    pub(crate) fn make_building(
        &mut self,
        player: usize,
        master: i16,
        unit_ai: Result<Option<u32>, Unsupported>,
        position: [f32; 3],
    ) -> Result<(i32, [f32; 3]), Unsupported> {
        let made = self.master_of(player, master)?.clone();
        let building = made.building.ok_or(Unsupported(
            "a building constructor of a master without its building part",
        ))?;
        let given_z = position[2];
        // the new object's new_position: the ground's height (slot 0x60)
        let position = self.on_ground(player, master, position);
        let id = self.next_object_id;
        self.next_object_id += 1;
        // what its first update as a built building starts (slot 0x7f0)
        let (default_make, default_task) = self
            .masters
            .get(player)
            .and_then(|m| m.get(master))
            .map_or((false, None), Master::default_start);
        let ai = unit_ai
            .and_then(|t| Ok((t, self.new_object(player, master, &made, position)?)))
            .map(|(t, new)| {
                t.and_then(|t| UnitAi::constructed(t, &new, self.streams.get(Stream::Ai)))
            });
        let ai = ai.unwrap_or_else(|e| {
            // the constructor's draws are unknown, so the turn player's are too
            crate::trace::player_unsupported(self.ticks, self.player_turn, e);
            let turn = &mut self.players[self.player_turn];
            let at = turn.objects.len();
            let _ = turn.unsupported.get_or_insert((self.ticks, at, e));
            turn.stopped = true;
            None
        });
        // the combat setup (FUN_14121df60): its class counters (FUN_141226060), its powerup
        // offset (`+0xa54`, the "setup" draw) and, for a building with a train location (an
        // entry of `+0x898` other than -1), its paid cost in attribute 247 (slot 0xf8)
        if let Some(m) = self.masters.get(player).and_then(|m| m.get(master)) {
            count_class(&mut self.players[player].attributes, m, 1.0);
        }
        let _ = self.streams.get(Stream::Object).rand15();
        if let Some(m) = self
            .masters
            .get(player)
            .and_then(|m| m.get(master))
            .filter(|m| m.kind == BUILDING_KIND && m.trained_at.iter().any(|&t| t != -1))
        {
            let paid: i32 = m
                .cost
                .iter()
                .filter(|&&(resource, _, paid)| 0 <= resource && paid)
                .map(|&(_, amount, _)| i32::from(amount))
                .sum();
            self.players[player].add_attribute(BUILDING_COST, paid as f32)?;
        }
        // the static init (FUN_1411a45a0): the last storage entry of flag 0 is held
        // (FUN_141193710), those of flags 1, 2 and 32 go to the owner (FUN_141193860)
        let storage = self
            .masters
            .get(player)
            .and_then(|m| m.get(master))
            .map(|m| m.storage.clone())
            .unwrap_or_default();
        let mut held = (-1, 0.0);
        for &(kind, amount, flag) in &storage {
            if flag == 0 && kind != -1 {
                held = (kind, amount);
            }
        }
        // what its death reads of the master: its dying sprite, death spawns and decay
        let death = self.masters.get(player).and_then(|m| m.get(master)).map_or(
            (None, false, -1, None),
            |m| {
                (
                    (0 <= m.death_sprite).then_some(m.death_sprite),
                    0 <= m.death_spawns[0],
                    m.death_spawns[1],
                    m.life,
                )
            },
        );
        let owner = &mut self.players[player];
        for &(kind, amount, flag) in &storage {
            if 0 <= kind && matches!(flag, 1 | 2 | 32) {
                owner.add_attribute(kind, amount)?;
            }
        }
        // a building that moves (a mule cart: its master's speed above 0) gets the moving
        // object's init (FUN_1411580a0: facing x, its right -z, not waiting); the other
        // buildings' movement state is never read
        let moving = self
            .masters
            .get(player)
            .and_then(|m| m.get(master))
            .and_then(|m| m.moving)
            .filter(|m| 0.0 < m.speed);
        let world_time = self.world_time;
        let mover = moving.map(|stats| Mover {
            forward: [1.0, 0.0, 0.0],
            right: [0.0, 0.0, crate::spawn::START_RIGHT_Z],
            velocity: None,
            paths: Vec::new(),
            factor: 1.0,
            stats,
            waiting: 0,
            wait_delays: 0,
            attack_moving: false,
            last_move: world_time,
            moved_at: position,
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
        });
        // the constructor (FUN_1411de5f0)
        let [x, y, _] = position;
        let (tx, ty) = (
            crate::checkpoint::cvttss2si(x),
            crate::checkpoint::cvttss2si(y),
        );
        let terrain = self.spatial.terrain.as_ref();
        // masters 50 and 1889 on snow keep their snow (+0xc61 2)
        let snow = matches!(master, FARM | PASTURE_1889)
            && -1 < tx
            && -1 < ty
            && terrain
                .map(|t| t.snow_under(tx, ty))
                .transpose()?
                .unwrap_or(false);
        // +0xcd0 (FUN_141188db0): mangrove or rice terrain under its place
        let rice = terrain.is_some_and(|t| matches!(t.kind(tx, ty), Some(54 | 55 | 63..=67)));
        let foundation = !matches!(made.class, 51 | 54)
            && !matches!(master, 357 | 1188)
            && building.head_unit == -1;
        let hp = if foundation && 0 < made.build_time {
            1.0
        } else {
            f32::from(made.hit_points)
        };
        let progress = if foundation {
            0.0
        } else {
            f32::from(made.build_time)
        };
        // FUN_1411cf770 with no master copy of its own and no annex link yet
        let desolid = 0 < made.build_time && !matches!(made.class, 51 | 54) && progress == 0.0;
        let owner = &mut self.players[player];
        owner.objects.push(Unit {
            id,
            footprint_waits: 0,
            position,
            life: Lifecycle {
                class: Class::Animated,
                state: 0,
                hp,
                master_id: master,
                attribute_type: held.0,
                amount: held.1,
                dying_sprite: death.0,
                death_spawn: death.1,
                remains: death.2,
                master: death.3,
                ..Lifecycle::default()
            },
            ai,
            artifact: None,
            unmodelled_action: None,
            production: None,
            gather_point: None,
            build: None,
            mover,
            move_to: None,
            fly: None,
            construction: Some(Construction {
                progress,
                desolid,
                builders: 0,
                hit_points: made.hit_points,
                build_time: made.build_time,
                record: crate::spatial::Obstruction {
                    kind: made.obstruction_type,
                    class: made.obstruction_class,
                },
                snow,
                sprite_override: false,
                started: false,
            }),
            work_rate_powerup: 0.0,
            attackable: true,
            untargetable: false,
            under_attack: false,
            last_human_order: 0,
            town_bell: crate::town_bell::TownBell::default(),
            pasture: -1,
            my_pasture: -1,
            undeletable: false,
            capture_flag: 0,
            annex_of: None,
            group: crate::group::GroupLink::default(),
            hp_powerups: [0.0; 2],
            hunt: None,
            gather: None,
            attack: None,
            explore: None,
            enter: None,
            missile: None,
            attack_count: 0,
            // the TRIBE combat setup (FUN_14121df60): the master's volley fire
            firepower: self
                .masters
                .get(player)
                .and_then(|m| m.get(master))
                .map_or(0.0, |m| m.volley_fire),
            next_volley: 0,
            reload_worldtime: 0,
            garrison: Vec::new(),
            garrison_time: 0,
            relics: [0; 2],
            livestock_summation: 0.0,
            raider_countdown: 0,
            // every building holds a gate's lock, open
            gate: Some(Gate {
                first_update: true,
                ..Gate::default()
            }),
            workers: 0,
            combat: true,
            default_task,
            default_fly: None,
            default_make,
            make: None,
            wait_solidify: None,
            stopped: false,
            sprite: None,
            // the animated object's constructor (FUN_141070cf0): animation speed (+0x7b8) 0
            anim_speed: Some(0.0),
            attack_timer: 0.0,
            abilities: crate::abilities::Abilities::default(),
            reload_stinger: 0.0,
            reload_powerup: 0.0,
            attack_powerup: 0.0,
            armor_powerups: [0; 2],
            armor_removed: [0; 2],
            charge: 0.0,
            charge_hit: false,
            idle_variant: 0,
            roo_called: false,
            rice,
            store: Vec::new(),
            own_master: None,
            dead_master: None,
            attacking_units: Vec::new(),
        });
        // the static init's add to its player (slot 0x2d0): it waits in the tracker's pending
        // lists
        self.track_add(player, id, master);
        self.building_place(player, id, &made, position, desolid);
        if foundation && 0 < made.build_time {
            self.construction_terrain(player, master, position)?;
        }
        self.clear_footprint(player, master, [position[0], position[1]], false)?;
        for (slot, &annex) in building.annexes.iter().enumerate() {
            if annex == -1 {
                continue;
            }
            // a master the player has not got makes nothing (its slot 0x40 is not called)
            let Some(am) = self.masters.get(player).and_then(|m| m.get(annex)).cloned() else {
                continue;
            };
            // the .dat's annexes are buildings (type 80): another type's master has no building
            // part, which the constructor below refuses
            let [ox, oy] = building.annex_offsets[slot];
            let ai = Ok(crate::unit_ai::factory_type(
                am.class,
                annex,
                am.volley_missile.unwrap_or(-1),
                am.combat_level,
            ));
            let at = [position[0] + ox, position[1] + oy, given_z];
            let (aid, at) = self.make_building(player, annex, ai, at)?;
            self.link_annex(player, id, aid, &made, &am)?;
            self.clear_footprint(player, annex, [at[0], at[1]], false)?;
        }
        self.watch_started(player, id)?;
        Ok((id, position))
    }

    /// The building constructor's link of a new annex `aid` (master `am`) to its building `id`
    /// (master `made`): the building's slot and the annex's `+0xbb8`; its owner's watched list
    /// lets the annex go (`FUN_141091310`) unless the building is a gate (39); a tall annex
    /// master makes it desolid (`FUN_1411cf770`, which an annex's link lets through: its record
    /// and mesh shape go, its own master copy has no height, the watched list lets it go; its
    /// tiles keep their links, as an annex's do).
    pub(in crate::turn) fn link_annex(
        &mut self,
        player: usize,
        id: i32,
        aid: i32,
        made: &Master,
        am: &Master,
    ) -> Result<(), Unsupported> {
        let not_held = crate::build::NOT_HELD;
        let (_, a) = self.object_mut(aid).ok_or(not_held)?;
        a.annex_of = Some(id);
        self.spatial.set_annex(aid, Some(id));
        if made.class != GATE {
            self.unwatch(player, aid);
        }
        if 0.0 < am.height {
            let (_, a) = self.object_mut(aid).ok_or(not_held)?;
            let c = a
                .construction
                .as_mut()
                .ok_or(crate::build::NO_CONSTRUCTION)?;
            if !c.desolid {
                c.desolid = true;
                self.pathing.removed(aid);
                self.spatial.drop_record(aid);
                self.spatial.set_tall(aid, false);
                self.pathing.tiles_changed(self.spatial.take_changes());
                self.unwatch(player, aid);
            }
        }
        Ok(())
    }

    /// BUILD's first step (`FUN_140e7a400`): a farm (master 50) whose point's tile (truncated) is
    /// mangrove shallows or forest (terrains 54, 55, `FUN_1413fc100`) or a rice farm's (63 to 67,
    /// `FUN_1413fc220`) is a rice farm (1187); a point off the map keeps the farm.
    pub(in crate::turn) fn farm_on_rice_terrain(&self, master: i16, position: [f32; 3]) -> i16 {
        if master != FARM {
            return master;
        }
        let (x, y) = (position[0] as i32, position[1] as i32);
        let terrain = self.spatial.terrain.as_ref().and_then(|t| t.kind(x, y));
        match terrain {
            Some(54 | 55 | 63..=67) => RICE_FARM,
            _ => master,
        }
    }

    /// BUILD's pending debit (`FUN_141630d10` with 0, before its placement test): every cost
    /// entry of the master, paid or not, by resource, comes off the player's pending debit.
    pub(in crate::turn) fn debit_listed(
        &mut self,
        player: usize,
        master: i16,
    ) -> Result<(), Unsupported> {
        let site = self
            .masters
            .get(player)
            .and_then(|m| m.get(master))
            .ok_or(Unsupported("BUILD of a master the player does not have"))?;
        let mut listed = [0i16; 4];
        for &(resource, amount, _) in &site.cost {
            if let Some(r) = usize::try_from(resource).ok().filter(|&r| r < 4) {
                listed[r] = listed[r].wrapping_add(amount);
            }
        }
        for (debit, amount) in self.players[player].pending_debit.iter_mut().zip(listed) {
            *debit -= f32::from(amount);
        }
        Ok(())
    }

    /// BUILD's builders' test (`FUN_140e7a400` after the pending debit): a human player's BUILD
    /// needs one unit of the packet that can build the master ([`Turns::builds`]; one the world
    /// holds, `FUN_140e6bad0`, follows from it), else it places nothing; a computer's passes.
    pub(in crate::turn) fn build_has_builder(
        &self,
        player: usize,
        master: i16,
        builders: &[i32],
    ) -> Result<bool, Unsupported> {
        if self.players[player].computer {
            return Ok(true);
        }
        let trained_at = self.master_of(player, master)?.trained_at.clone();
        for &id in builders {
            if self.builds((master, &trained_at), id)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Whether BUILD (`FUN_140e7a400`) places a foundation of `master` at `at` for `player`: its
    /// placement test ([`crate::spawn::build_placement`], the master's slot 0x50), then the
    /// payment's (`FUN_14162fb60` -> `FUN_141618db0`: every cost entry of a resource, paid or
    /// not, at most what the player holds). A refused BUILD places, pays and draws nothing (the
    /// player's slot 0x380 shows why). A cost in population (attribute 4) is not modelled.
    pub(in crate::turn) fn build_allowed(
        &self,
        player: usize,
        master: i16,
        at: [f32; 2],
    ) -> Result<bool, Unsupported> {
        let human = !self.players[player].computer;
        if self.placement_code(player, master, at, human)? != 0 {
            return Ok(false);
        }
        self.affords(player, master)
    }

    /// The master's placement test (slot 0x50, [`crate::spawn::build_placement`]) of a
    /// foundation of `master` for `player` at `at`, `human` its explored test (`p6`): 0 when it
    /// may go there, else the game's code.
    pub(crate) fn placement_code(
        &self,
        player: usize,
        master: i16,
        at: [f32; 2],
        human: bool,
    ) -> Result<u8, Unsupported> {
        let site = self.master_of(player, master)?;
        if self.spatial.sight.is_none() {
            return Err(Unsupported(
                "BUILD's placement test without the players' sight",
            ));
        }
        let restriction = self
            .spatial
            .terrain
            .as_ref()
            .and_then(|t| {
                usize::try_from(site.terrain_restriction)
                    .ok()
                    .and_then(|r| t.restrictions.get(r))
            })
            .ok_or(Unsupported(
                "a foundation's terrain restriction the data file does not have",
            ))?;
        let build = crate::spawn::BuildSite {
            player,
            human,
            clearance: site.clearance,
            class: site.class,
            obstructs: !matches!(site.obstruction_type, 0 | 12),
            restriction,
            hill_mode: site.hill_mode,
            placement: site.placement,
            zones: false,
        };
        let map = crate::spawn::WorldMap { turns: self };
        crate::spawn::build_placement(&build, &map, at)
    }

    /// The payment's test of a foundation of `master` for `player` (`FUN_14162fb60` ->
    /// `FUN_141618db0`): every cost entry of a resource, paid or not, at most what the player
    /// holds. A cost in population (attribute 4) is not modelled.
    pub(crate) fn affords(&self, player: usize, master: i16) -> Result<bool, Unsupported> {
        let site = self.master_of(player, master)?;
        let owner = &self.players[player];
        for &(resource, amount, _) in &site.cost {
            let Ok(r) = usize::try_from(resource) else {
                continue;
            };
            if r == 4 {
                return Err(Unsupported("a BUILD whose cost lists the population"));
            }
            let held = if r < 4 {
                owner.resources[r]
            } else {
                owner.attributes.get(r).copied().unwrap_or(0.0)
            };
            if held < f32::from(amount) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// `FUN_14123bb20`'s sending off (with `DAT_1439e60e2` 1) of the units on `foundation`'s
    /// footprint: each of type 30 or more in state 2 whose top action is not on the foundation,
    /// that stands (`FUN_14114f740`: Gaia's of classes 9 and 10, or not walking, its animation
    /// speed `+0x7b8` at most 0, and its path level `+0x858` absent or in state 0 or 6) and
    /// whose unit AI's action is not 0x2bf or 0x2c0 (moving off already) is sent off: its unit AI
    /// gets notify 0x216 (slot 0x38) with the foundation master's radii times 100 (truncated) and
    /// the foundation ([`crate::unit_ai::SENT_OFF`]). The foundation owner's own units of type 70
    /// and more of a formation category (`+0x8bc`) would go together to its farthest free spot
    /// (`FUN_1411566b0` with 1, `FUN_1416123a0`'s group move, then 0x21b each), which is not
    /// modelled.
    pub(in crate::turn) fn evacuate(
        &mut self,
        foundation: i32,
        units: &[i32],
    ) -> Result<(), Unsupported> {
        /// The notify's radii are hundredths (`DAT_14330ebcc`).
        const RADIUS_SCALE: f32 = crate::image::unit_ai::RADIUS_SCALE;
        let (site_owner, site) = self.object(foundation).ok_or(Unsupported(
            "a footprint's foundation the slice does not hold",
        ))?;
        let radius = self.master_of(site_owner, site.life.master_id)?.radius;
        let p = [
            (radius[0] * RADIUS_SCALE) as i32,
            (radius[1] * RADIUS_SCALE) as i32,
            foundation,
        ];
        let mut together = Vec::new();
        for &id in units {
            let Some((owner, u)) = self.object(id) else {
                continue;
            };
            let master = self.master_of(owner, u.life.master_id)?;
            if master.kind < 30 || u.life.state != 2 || u.top_target() == Some(foundation) {
                continue;
            }
            let walking = u.anim_speed.is_some_and(|s| 0.0 < s);
            let level = u
                .mover
                .as_ref()
                .and_then(|m| m.paths.first())
                .map(|l| l.state);
            let stands = (owner == 0 && matches!(master.class, 9 | 10))
                || (!walking && level.is_none_or(|s| matches!(s, 0 | 6)));
            let busy = u.ai.as_ref().is_none_or(|ai| {
                matches!(
                    ai.current_action,
                    crate::unit_ai::MOVE_ASIDE_ACTION | crate::unit_ai::GROUP_ASIDE_ACTION
                )
            });
            if !stands || busy {
                continue;
            }
            if owner == site_owner && 70 <= master.kind && master.formation_category != 0 {
                together.push(id);
                continue;
            }
            let state = u.life.state;
            let Some(crate::order::Place::Live(q, i)) = self.place(id) else {
                return Err(Unsupported("a sleeping unit sent off a footprint"));
            };
            if let Some(ai) = self.players[q].objects[i].ai.as_mut() {
                ai.post(Notify::own(id, crate::unit_ai::SENT_OFF, p), state)?;
            }
        }
        if !together.is_empty() {
            self.evacuate_together(foundation, radius, &together)?;
        }
        Ok(())
    }

    /// `FUN_14123bb20`'s owner's units of a formation category (`+0x8bc`) on `foundation`'s
    /// footprint (master radii `radius`), in the tiles' order: the farthest free spot beside the
    /// footprint from the first of them (`FUN_1411566b0` with 1, [`Spatial::spot_off`]); then, with
    /// one, `FUN_1416123a0`: the order splitter on them ([`Turns::loose_units`]: a unit group it
    /// forms, its group move `FUN_14100efd0`, is not modelled) and each unit it leaves loose takes
    /// the action of moving off a footprint (`0x2bf`) and moves to the spot at its own height
    /// (slot 0x138 without a target); last each of them whose unit AI's action is not `0x2bf` gets
    /// notify `0x21b` ([`crate::unit_ai::SENT_OFF_GROUP`]).
    pub(in crate::turn) fn evacuate_together(
        &mut self,
        foundation: i32,
        radius: [f32; 2],
        units: &[i32],
    ) -> Result<(), Unsupported> {
        let site_at = self
            .spatial
            .get(foundation)
            .ok_or(Unsupported("a footprint's foundation without a place"))?
            .position;
        let (_, first) = self
            .object(units[0])
            .ok_or(Unsupported("a unit sent off that is gone"))?;
        let mover = first
            .mover
            .as_ref()
            .ok_or(Unsupported("a unit sent off without movement state"))?;
        let spot = self.spatial.spot_off(
            first.position,
            (first.id, mover.stats.radius[0]),
            mover.stats.restriction_mask,
            (site_at, radius),
            true,
        )?;
        let Some([x, y]) = spot else {
            return Ok(());
        };
        let loose = self.loose_units(units)?;
        for id in loose {
            let Some(crate::order::Place::Live(q, i)) = self.place(id) else {
                continue;
            };
            let z = self.players[q].objects[i].position[2];
            if let Some(ai) = self.players[q].objects[i].ai.as_mut() {
                ai.set_action(crate::unit_ai::MOVE_ASIDE_ACTION);
            }
            self.move_point_order(q, i, [x, y, z])?;
        }
        for &id in units {
            let Some(crate::order::Place::Live(q, i)) = self.place(id) else {
                continue;
            };
            let unit = &mut self.players[q].objects[i];
            let state = unit.life.state;
            if let Some(ai) = unit.ai.as_mut()
                && ai.current_action != crate::unit_ai::MOVE_ASIDE_ACTION
            {
                ai.post(
                    Notify::own(id, crate::unit_ai::SENT_OFF_GROUP, [0, 0, 0]),
                    state,
                )?;
            }
        }
        Ok(())
    }

    /// The building constructor's terrain (`FUN_1411de5f0`, for a master with a build time, not
    /// of class 51 or 54, nor 357 or 1188): a farm (master 50) lays terrain 29 under its
    /// clearance, a rice farm (1187) terrain 65 (`FUN_1411ce250` -> `FUN_1411ddd90`,
    /// [`crate::terrain::Terrain::cover`]), before the footprint clearing.
    pub(in crate::turn) fn construction_terrain(
        &mut self,
        player: usize,
        master: i16,
        position: [f32; 3],
    ) -> Result<(), Unsupported> {
        /// The farms and the terrain their foundations lay (`0x1d`, `0x41`).
        const LAID: [(i16, u8); 2] = [(50, 29), (1187, 65)];
        let Some(&(_, laid)) = LAID.iter().find(|(m, _)| *m == master) else {
            return Ok(());
        };
        let m = self.master_of(player, master)?;
        if m.build_time <= 0 {
            return Ok(());
        }
        let f = crate::terrain::Foundation {
            position,
            clearance: m.clearance,
            terrain: laid,
            snow: false,
            master,
            class: m.class,
            terrain_bonus: m.terrain_bonus,
            mode_680: self.mode_680,
        };
        self.spatial
            .terrain
            .as_mut()
            .ok_or(Unsupported("a farm's foundation without the map's terrain"))?
            .cover(&f)
    }

    /// The footprint clearing (`FUN_1415c3620` with `param_7` 0) of a new building of `master`
    /// of `player` at `at`: the building constructor's (`FUN_1411de5f0`, `trees` false) and a new
    /// object's (`FUN_14162ce00` with its flag, `trees` true). The objects it takes
    /// ([`crate::build::footprint_clearing`]), each in the tiles' order: [`Turns::clear_object`].
    /// Nothing for a master the player has not got.
    ///
    /// # Errors
    ///
    /// What the choice and the deletes do not model.
    pub(crate) fn clear_footprint(
        &mut self,
        player: usize,
        master: i16,
        at: [f32; 2],
        trees: bool,
    ) -> Result<(), Unsupported> {
        let Some(m) = self.masters.get(player).and_then(|m| m.get(master)) else {
            return Ok(());
        };
        let site = crate::build::ClearingSite {
            player,
            master,
            class: m.class,
            radius: m.radius,
            at,
            trees,
        };
        let look = |id: i32| self.clear_look(id);
        let cleared = crate::build::footprint_clearing(&self.spatial, &site, &look)?;
        for cl in cleared {
            self.clear_object(cl, false)?;
            if !trees {
                continue;
            }
            // FUN_141189670 after its destroy: unless +0x686 is set, a tall master's (COMISS: a
            // NaN height too) or a recorded object's record goes; then +0x686 (FUN_141189b60)
            let look = self.clear_look(cl.id).ok_or(Unsupported(
                "a footprint clearing of an object the slice does not hold",
            ))?;
            let recorded = self
                .spatial
                .get(cl.id)
                .is_some_and(|o| o.obstruction.is_some());
            if !look.roo_called && (crate::fmath::below_or_unordered(0.0, look.height) || recorded)
            {
                self.pathing.removed(cl.id);
                self.spatial.drop_record(cl.id);
                self.pathing.tiles_changed(self.spatial.take_changes());
            }
            if let Some((_, u)) = self.object_mut(cl.id) {
                u.roo_called = true;
            }
        }
        Ok(())
    }

    /// What the footprint clearing reads of object `id` ([`crate::build::ClearLook`]).
    pub(crate) fn clear_look(&self, id: i32) -> Option<crate::build::ClearLook> {
        let (p, u) = self.object(id)?;
        clear_look_of(u, self.masters.get(p))
    }

    /// An object the footprint clearing deletes ([`crate::build::Cleared`]): a wall first goes
    /// through the building's delete core (`FUN_1411ccbb0`, [`Turns::remove_building`]), then
    /// its destroy (slot 0xd0, [`Turns::destroy`]). The builder's clearing (`roo_done`) dropped
    /// its record in the Build's update already (`FUN_141189670`): `+0x686` is set
    /// (`FUN_141189b60` with 1).
    ///
    /// # Errors
    ///
    /// An object the slice does not hold, and what the deletes do not model.
    pub(crate) fn clear_object(
        &mut self,
        cl: crate::build::Cleared,
        roo_done: bool,
    ) -> Result<(), Unsupported> {
        let (owner, _) = self.object(cl.id).ok_or(Unsupported(
            "a footprint clearing of an object the slice does not hold",
        ))?;
        if cl.wall {
            self.remove_building(owner, cl.id)?;
        }
        self.destroy(owner, cl.id)?;
        if roo_done && let Some((_, u)) = self.object_mut(cl.id) {
            u.roo_called = true;
        }
        Ok(())
    }

    /// A new building's place (its static init's `new_position`): it takes its place on the map,
    /// with its obstruction record unless it is desolid (`FUN_1411cf770` drops the one its place
    /// gave it; its first builder brings it back), and its sight, which waits for its
    /// construction to start (its place links its tiles to its anchor, `FUN_1411e6780`, and its
    /// desolid flag unlinks them).
    pub(in crate::turn) fn building_place(
        &mut self,
        player: usize,
        id: i32,
        made: &Master,
        position: [f32; 3],
        desolid: bool,
    ) {
        let record = crate::spatial::Obstruction {
            kind: made.obstruction_type,
            class: made.obstruction_class,
        };
        let placed = Placed {
            position,
            radius: made.radius,
            class: made.class,
            player,
            master: made.id,
            obstruction_type: made.obstruction_type,
            buildable_on: made.buildable_on,
            tall: made.tall,
            speed: 0.0,
            unknown: None,
            obstruction: (!desolid && record.recorded()).then_some(record),
        };
        self.spatial.insert(id, placed);
        if placed.obstruction.is_some() {
            self.pathing.moved(id, &placed, false);
            self.pathing.tiles_changed(self.spatial.take_changes());
        }
        if let Some(v) = self.spatial.sight.as_mut() {
            let umv = v.link_building(position, made.radius);
            if desolid {
                v.unlink_building(position, made.radius);
            }
            v.add_seer(
                id,
                Seer {
                    player,
                    master: made.id,
                    los: made.line_of_sight,
                    state: 0,
                    explored: false,
                    building: Some(BuildingSight::default()),
                    umv: umv as u8,
                    capture: capture_kind(player, made.class, made.id),
                    kind: 80,
                },
            );
        }
    }

    /// BUILD's builders (the cost was paid with the foundation, [`Turns::found`]; the pending
    /// debit came before, [`Turns::debit_listed`]), the handler's last loop: each unit of the
    /// packet the world holds (`FUN_1415417a0`: any player's, asleep or not), in the packet's
    /// order, of a master of type 30 and up that moves (its slot 0xe8; COMISS: a NaN speed too),
    /// or any unit of a queued BUILD: its ignore list empties (`FUN_141197da0`); with a unit AI,
    /// a queued BUILD, or one that `commits` while the AI holds an order not ready
    /// (`FUN_140c4dfa0`), goes into its queue as the build order (slot 0x30 into slot 0x28:
    /// [`Turns::queue_build`], issued by the unit's own player); else its slot 0x128 on the
    /// foundation: a static object's (types below 40, `FUN_141071bd0`) does nothing, a unit's
    /// takes the Build ([`Turns::build_task`], [`Turns::build_directly`]), another player's
    /// whatever task its slot 0x7e8 finds on the foundation ([`Turns::order_work`]). A unit the world no
    /// longer holds is passed over. Not modelled: the readiness pass of a committing BUILD with
    /// orders queued (`FUN_140c42b60`).
    pub(in crate::turn) fn task_builders(
        &mut self,
        player: usize,
        foundation: i32,
        master: i16,
        position: [f32; 3],
        builders: &[i32],
        (queued, commits): (bool, bool),
    ) -> Result<(), Unsupported> {
        // the builders go through the order splitter, which counts the command
        self.groups.count_command();
        let site = (player, foundation, master);
        for &id in builders {
            let Some((q, unit)) = self.object(id) else {
                continue;
            };
            let m = self.master_of(q, unit.life.master_id)?;
            let speed = m.moving.map_or(0.0, |s| s.speed);
            let kind = m.kind;
            let moves = 0x1d < kind && crate::fmath::below_or_unordered(0.0, speed);
            if !moves && !queued {
                continue;
            }
            let to_queue = unit
                .ai
                .as_ref()
                .is_some_and(|ai| queued || (commits && ai.has_unready_order()));
            let Some(crate::order::Place::Live(q, idx)) = self.place(id) else {
                // a sleeping object is a static one without a unit AI: its slot 0x128 does
                // nothing
                continue;
            };
            if to_queue {
                self.queue_build(q, idx, site, position)?;
                if commits {
                    return Err(Unsupported(
                        "the readiness pass of a committing BUILD with orders queued (FUN_140c42b60)",
                    ));
                }
            } else if STATIC_TYPES_END <= kind && q == player {
                let task = self.build_task(q, id, site, position)?;
                self.build_directly(q, idx, task, site, position)?;
            } else if STATIC_TYPES_END <= kind {
                // another player's unit: slot 0x128's slot 0x7e8 finds whatever task it has on
                // the foundation (a Build on an ally's), the work command's general path
                let work = crate::order::Work {
                    target: foundation,
                    point: [position[0], position[1], 0.0],
                    carry_test: false,
                    filter: -1,
                    notified: true,
                };
                self.order_work(q, idx, &work)?;
            } else if let Some(m) = self.players[q].objects[idx].mover.as_mut() {
                m.ignore_units.clear();
            }
        }
        Ok(())
    }

    /// What slot 0x128 on `foundation` (of `master`, player `fp`'s, at `position`) takes for the
    /// builder `id` of player `q`: the master it builds with ([`Masters::work_master`], with
    /// `q`'s masters and attributes) and the Build action of that master's task. A builder
    /// whose action would be replaced, without a Build task for it or with one not enabled, is
    /// not modelled.
    pub(crate) fn build_task(
        &self,
        q: usize,
        id: i32,
        (fp, foundation, master): (usize, i32, i16),
        position: [f32; 3],
    ) -> Result<(i16, BuildAction), Unsupported> {
        let masters = self
            .masters
            .get(q)
            .ok_or(Unsupported("BUILD tasks the builders: no master table"))?;
        let site = self
            .masters
            .get(fp)
            .and_then(|m| m.get(master))
            .ok_or(Unsupported("BUILD of a master the player does not have"))?;
        let target = TaskTarget {
            id: foundation,
            master: site,
            player: fp,
            state: 0,
            held: (-1, 0.0),
            hp: 1.0,
            max_hp: f32::from(site.hit_points),
            attackable: true,
            pasture: -1,
            top_action: None,
        };
        let world = World {
            players: &self.players,
            drop_sites: &self.drop_sites,
        };
        let owner = &world.players[q];
        let unit = owner
            .objects
            .iter()
            .find(|o| o.id == id)
            .ok_or(Unsupported("a builder the slice does not have"))?;
        if unit.artifact.is_some() || unit.production.is_some() || unit.fly.is_some() {
            return Err(Unsupported("a builder with an action (its replacement)"));
        }
        let actor = Actor {
            id,
            master: masters
                .get(unit.life.master_id)
                .ok_or(Unsupported("a unit of a master the player does not have"))?,
            player: q,
            held: (unit.life.attribute_type, unit.life.amount),
            attribute_95: owner.attributes.get(95).copied().unwrap_or(0.0),
            civ: owner.civ,
        };
        let (with, task) = masters
            .work_master(&actor, &target, &target, -1, &world)?
            .ok_or(Unsupported("a builder with no task for its foundation"))?;
        if task.action_type != BUILD {
            return Err(Unsupported(
                "a builder whose task for its foundation is not Build",
            ));
        }
        if !crate::tasks::enabled(&owner.attributes, task.enablement) {
            return Err(Unsupported("a builder whose Build task is not enabled"));
        }
        let builder = masters
            .get(with)
            .expect("work_master returns a master the player has");
        let gap = footprint_gap(unit.position, builder.radius, position, site.radius);
        let task = BuildTask {
            range: task.work_range,
            work_value: task.work_val1,
            work_rate: builder.work_rate + unit.work_rate_powerup,
        };
        Ok((with, BuildAction::start(foundation, gap, task)))
    }

    /// The queue path of a builder at `idx` of player `q` (slot 0x30 into slot 0x28): its
    /// ignore list empties (`FUN_141197da0`) and its unit AI's queue takes the build order on
    /// `foundation` (of `master`, player `fp`'s) at `position`, issued by `q`, priority 100, the
    /// point shown, ready, as the game's saves hold it ([`Turns::build_order`] runs it).
    pub(crate) fn queue_build(
        &mut self,
        q: usize,
        idx: usize,
        (fp, foundation, master): (usize, i32, i16),
        position: [f32; 3],
    ) -> Result<(), Unsupported> {
        let unit = &mut self.players[q].objects[idx];
        if let Some(m) = unit.mover.as_mut() {
            m.ignore_units.clear();
        }
        let state = unit.life.state;
        let order = crate::unit_ai::QueuedOrder {
            issuer: q as i32,
            order: BUILD_ORDER,
            priority: 100,
            target: foundation,
            target_owner: fp as i32,
            point: [position[0], position[1], 0.0],
            range: 0.0,
            shown: [position[0], position[1]],
            ready: true,
            target_type: i32::from(master),
        };
        let ai = unit
            .ai
            .as_mut()
            .ok_or(Unsupported("a queued build order without a unit AI"))?;
        let _ = ai.queue_order(state, order, false, false)?;
        Ok(())
    }

    /// The direct path of a builder at `idx` of player `q`, slot 0x128 on `foundation` (player
    /// `fp`'s) at `position` with what [`Turns::build_task`] found: its ignore list empties
    /// (`FUN_141197da0`), the town-bell record (slot 0x7e8 with task filter −1), then the work
    /// command (`FUN_14106ff70`): the master changes, the Build takes its foundation, the unit
    /// AI's notify, the action list clears and holds the Build.
    pub(crate) fn build_directly(
        &mut self,
        player: usize,
        idx: usize,
        (with, build): (i16, BuildAction),
        (fp, foundation, master): (usize, i32, i16),
        position: [f32; 3],
    ) -> Result<(), Unsupported> {
        let site_class = i32::from(self.master_of(fp, master)?.class);
        if let Some(m) = self.players[player].objects[idx].mover.as_mut() {
            m.ignore_units.clear();
        }
        // slot 0x128 -> slot 0x7e8 (task filter -1): the villager's town-bell record, its
        // flag cleared (FUN_1412067f0)
        self.town_bell_order(player, idx, foundation, -1)?;
        // the work command (FUN_14106ff70): the master changes (slot 0x90, its action list
        // cleared); the Build action takes its foundation (FUN_140a7a230, slot 0x178)
        if self.players[player].objects[idx].life.master_id != with {
            self.change_master(player, idx, with)?;
        }
        self.take(foundation, self.releaser(player, idx));
        // the work command's notify (slot 0x828): the build order and the foundation, with
        // the point slot 0x128 passes (BUILD's x and y, on the ground)
        let unit = &mut self.players[player].objects[idx];
        if let Some(ai) = unit.ai.as_mut() {
            ai.notify_work(
                BUILD_ORDER,
                BUILD_ACTION,
                Some((foundation, site_class)),
                [position[0], position[1], 0.0],
                true,
                self.streams.get(Stream::Ai),
            )?;
        }
        // the install (slot 0x808): the action list clears (its actions' teardowns), then
        // holds the Build
        self.clear_actions(player, idx)?;
        self.players[player].objects[idx].build = Some(build);
        Ok(())
    }

    /// The build order (`0x2be`) a unit AI's queue runs (slot 0x230's case: slot 0xf0 with 1 on its
    /// foundation, [`Turns::build_object`]), then the tail takes the order (702) with its
    /// priority.
    pub(crate) fn build_order(
        &mut self,
        player: usize,
        idx: usize,
        order: crate::unit_ai::QueuedOrder,
    ) -> Result<(), Unsupported> {
        self.build_object(player, idx, order.target)?;
        let ai = self.players[player].objects[idx]
            .ai
            .as_mut()
            .ok_or(Unsupported("a build order without a unit AI"))?;
        // the tail (`LAB_140c5d432`)
        ai.current_order = BUILD_ORDER;
        ai.order_priority = order.priority;
        Ok(())
    }

    /// `BuildObject` (the unit AI's slot 0xf0, `FUN_140c47d80`, with 1) on `foundation`: the
    /// unit's build command ([`Turns::build_command`]), then the AI's action becomes the build
    /// action (602; the last one kept) and its target the foundation (the last one kept) at its
    /// place with its master's class. Unlike BUILD's own path neither the town-bell record nor
    /// the work's notify.
    pub(crate) fn build_object(
        &mut self,
        player: usize,
        idx: usize,
        foundation: i32,
    ) -> Result<(), Unsupported> {
        let (class, f_at) = self.build_command(player, idx, foundation)?;
        let ai = self.players[player].objects[idx]
            .ai
            .as_mut()
            .ok_or(Unsupported("a BuildObject without a unit AI"))?;
        // FUN_140c47d80
        ai.last_action = ai.current_action;
        ai.current_action = BUILD_ACTION;
        ai.target_point = f_at;
        ai.last_target = ai.current_target;
        ai.last_target_type = ai.target_type;
        ai.current_target = foundation;
        ai.target_type = class;
        Ok(())
    }

    /// The unit's slot 0x388 (`FUN_141226fc0`) after its unit AI's update, with what it found
    /// there ([`Wait`]): slot 0x820's switch to the master its swap group gave (slot 0x90 with 0,
    /// [`Turns::change_master`]; a villager's candidates are of its own type, never a
    /// building's), the action list cleared (`FUN_140a98620`, [`Turns::clear_actions`]; a wait
    /// it held goes too, its teardown the base one's) and the new wait in it (slot 0x808,
    /// `FUN_14106e990`: the wait's slot 0x70 with no action and its start both do nothing).
    pub(crate) fn wait_for_foundation(
        &mut self,
        player: usize,
        idx: usize,
        wait: Wait,
    ) -> Result<(), Unsupported> {
        if let Some(master) = wait.switch {
            self.change_master(player, idx, master)?;
        }
        self.clear_actions(player, idx)?;
        self.players[player].objects[idx].wait_solidify = Some(
            crate::wait_solidify::WaitSolidify::new(wait.task, wait.target),
        );
        Ok(())
    }

    /// The unit's build command (slot 0x380, `FUN_14120d410`, forced) on `foundation`: a Build on
    /// top takes the foundation (its slot 0x48, [`Turns::build_take`]); without one its slot
    /// 0x820 with any action type and the switch ([`task_search`] on the foundation and its main
    /// building; the switch, slot 0x90 with 0, [`Turns::change_master`]) finds a task, and a
    /// Build action on it (`FUN_140aad4e0` with that task, whatever its type) replaces the
    /// unit's actions. The foundation's master's class and its place, for the unit AI. A
    /// foundation that is gone or without a task (slot 0x380's 0) is not modelled.
    pub(crate) fn build_command(
        &mut self,
        player: usize,
        idx: usize,
        foundation: i32,
    ) -> Result<(i32, [f32; 3]), Unsupported> {
        if self.players[player].objects[idx].top_action_kind() == Some(BUILD) {
            let class = self.build_take(player, idx, foundation)?;
            let f_at = self
                .object(foundation)
                .map(|(_, f)| f.position)
                .ok_or(Unsupported("a BuildObject on a foundation that is gone"))?;
            return Ok((class, f_at));
        }
        let gone = Unsupported("a build order on a foundation that is gone");
        let (fp, f) = self.object(foundation).ok_or(gone)?;
        let (site, f_at) = (self.master_of(fp, f.life.master_id)?, f.position);
        let find = |id: i32| self.object(id);
        let t = task_target_with(foundation, 1, &find, &self.masters)?.ok_or(gone)?;
        let general = task_target_with(foundation, 2, &find, &self.masters)?.ok_or(gone)?;
        let masters = self
            .masters
            .get(player)
            .ok_or(Unsupported("a build order: no master table"))?;
        let world = World {
            players: &self.players,
            drop_sites: &self.drop_sites,
        };
        let owner = &world.players[player];
        let unit = &owner.objects[idx];
        let actor = Actor {
            id: unit.id,
            master: masters
                .get(unit.life.master_id)
                .ok_or(Unsupported("a unit of a master the player does not have"))?,
            player,
            held: (unit.life.attribute_type, unit.life.amount),
            attribute_95: owner.attributes.get(95).copied().unwrap_or(0.0),
            civ: owner.civ,
        };
        // slot 0x820 with any action type (0xffff): whatever task it finds, the Build takes
        // (FUN_140aad4e0 on it), its type and its enablement unread
        let (task, switch) = task_search(masters, &actor, &t, &general, -1, &world)?
            .ok_or(Unsupported("a build order with no task for its foundation"))?;
        let builder = switch.unwrap_or(actor.master);
        let gap = footprint_gap(unit.position, builder.radius, f_at, site.radius);
        let build = BuildAction::start(
            foundation,
            gap,
            BuildTask {
                range: task.work_range,
                work_value: task.work_val1,
                work_rate: builder.work_rate + unit.work_rate_powerup,
            },
        );
        let class = i32::from(site.class);
        if let Some(m) = switch.map(|m| m.id) {
            self.change_master(player, idx, m)?;
        }
        self.take(foundation, self.releaser(player, idx));
        self.clear_actions(player, idx)?;
        self.players[player].objects[idx].build = Some(build);
        Ok((class, f_at))
    }

    /// SET GATHER POINT's handler ([`Command::GatherPoint`]).
    pub(in crate::turn) fn gather_point(
        &mut self,
        player: usize,
        ids: &[i32],
        [x, y]: [f32; 2],
        (target, target_master): (i32, i16),
        own: bool,
    ) -> Result<(), Unsupported> {
        for &id in ids {
            // another player's object (the game clears its id) or one that is gone
            let Some(i) = self.players[player].objects.iter().position(|u| u.id == id) else {
                continue;
            };
            let unit = &self.players[player].objects[i];
            let master = self
                .masters
                .get(player)
                .and_then(|m| m.get(unit.life.master_id))
                .ok_or(Unsupported(
                    "SET GATHER POINT on a master the slice does not have",
                ))?;
            if master.garrison == 0 {
                return Err(Unsupported(
                    "SET GATHER POINT on a building that does not garrison (its train list)",
                ));
            }
            let z = unit.gather_point.map_or(0.0, |g| g.point[2]);
            let gather = if own {
                Some(GatherPoint {
                    point: [unit.position[0], unit.position[1], z],
                    target: unit.id,
                    target_master: unit.life.master_id,
                })
            } else if target == -1 && x == -1.0 && y == -1.0 {
                None
            } else {
                Some(GatherPoint {
                    point: [x, y, z],
                    target,
                    target_master,
                })
            };
            let unit = &mut self.players[player].objects[i];
            unit.gather_point = gather;
            // the unit AI's slot 0x210 (FUN_140c16790): its command queue empties
            if let Some(ai) = unit.ai.as_mut() {
                ai.building_orders.clear();
            }
        }
        Ok(())
    }

    /// `BUILDING_COMMAND_QUEUE`'s handler (`0x140e64310`) for `player`: each object of `ids`
    /// the world holds with a unit AI queues the step (its slot 0x208, `FUN_140c15740`: the
    /// player, the order `0x2db` on a target or `0x2c6` on the point, priority −1, the target,
    /// its owner and class, the point with height −1, range 0, not ready). A building first puts
    /// its gather point in the queue when it has one, the queue is empty (slot 0x218) and the
    /// point is not on itself, then the gather point clears (`FUN_1411c7ff0` with −1). A target
    /// the world does not hold queues the point alone. The view's marker (`FUN_1414cd6d0`) is not
    /// held.
    pub(crate) fn command_queue(
        &mut self,
        player: usize,
        ids: &[i32],
        target: i32,
        [x, y]: [f32; 2],
    ) -> Result<(), Unsupported> {
        let step = |turns: &Self,
                    t: i32,
                    at: [f32; 2]|
         -> Result<crate::unit_ai::QueuedOrder, Unsupported> {
            let named = if t == -1 { None } else { turns.object(t) };
            let (target, owner, class) = match named {
                Some((tp, u)) => (
                    t,
                    tp as i32,
                    i32::from(turns.master_of(tp, u.life.master_id)?.class),
                ),
                None => (-1, -1, -1),
            };
            Ok(crate::unit_ai::QueuedOrder {
                issuer: player as i32,
                order: if target < 0 {
                    crate::unit_ai::MOVE_ORDER
                } else {
                    crate::unit_ai::ORDER_ON_TARGET
                },
                priority: -1,
                target,
                target_owner: owner,
                point: [at[0], at[1], -1.0],
                range: 0.0,
                shown: [-1.0, -1.0],
                ready: false,
                target_type: class,
            })
        };
        for &id in ids {
            let Some(crate::order::Place::Live(p, i)) = self.place(id) else {
                if self.place(id).is_some() {
                    return Err(Unsupported("a command queue on a sleeping object"));
                }
                continue;
            };
            let unit = &self.players[p].objects[i];
            let Some(ai) = unit.ai.as_ref() else {
                continue;
            };
            let building = self.master_of(p, unit.life.master_id)?.kind == crate::order::BUILDING;
            let mut queued = Vec::new();
            if building
                && let Some(g) = unit.gather_point
                && ai.building_orders.is_empty()
                && g.target != unit.id
            {
                // the gather point's own target id stays even when the world lost it
                let mut first = step(self, g.target, [g.point[0], g.point[1]])?;
                if first.target == -1 && g.target != -1 {
                    first.target = g.target;
                    first.order = crate::unit_ai::ORDER_ON_TARGET;
                }
                queued.push(first);
            }
            queued.push(step(self, target, [x, y])?);
            let unit = &mut self.players[p].objects[i];
            if building {
                unit.gather_point = None;
            }
            if let Some(ai) = unit.ai.as_mut() {
                ai.building_orders.extend(queued);
            }
        }
        Ok(())
    }

    /// RESEARCH (`0x140e7e270`) for `player`: a target other than −1, or a technology the tree
    /// has not got, does nothing. The costs it lists (`FUN_1413d4a00`) come off the pending
    /// debit (`FUN_141630d10`). The building is the packet's (the least loaded of several,
    /// `FUN_1411d0e90`, and the shared kind's every building, `FUN_1411c3fd0`, are not modelled);
    /// a building of master type 80 alive (state 2) whose master is among the technology's
    /// locations, with the technology in state 1 or repeatable, and without a top action of
    /// 0x7b to 0x7d (`FUN_141227a00`), queues it ([`Turns::queue_research`]); queued, its unit AI
    /// takes the action value 0x26c. A technology of `linkedTechs.json` ([`Turns::linked_techs`],
    /// the game settings' table `FUN_140f39f40` looks up) is not modelled (`FUN_140f3a0f0`,
    /// `FUN_140f39e80`). Not held: the interface's view and selection (`FUN_140e6de20`), the
    /// local player's statistics flag. Not modelled: the computer players' parts (`player+0x7a8`, `FUN_1411d5520`), the player's
    /// `+0x648` (0 for every player probed), Castle Age's flag (tech 0x66, game type 0x10).
    pub(in crate::turn) fn research(
        &mut self,
        player: usize,
        r: &ResearchAt,
    ) -> Result<(), Unsupported> {
        let owner = &self.players[player];
        if r.target != -1 || r.tech < 0 || owner.techs.len() <= r.tech as usize {
            return Ok(());
        }
        let tech = r.tech as usize;
        let def = self
            .techs
            .get(tech)
            .ok_or(Unsupported("a technology the data file has not got"))?;
        let get = |i: i16| crate::tech::Attributes::get(owner, i);
        let factors = crate::tech::factors(r.tech, &get)?;
        let listed = owner.techs[tech].listed_costs(def, factors);
        let mut sums = [0i16; 4];
        for (attribute, amount) in listed {
            if let Some(k) = usize::try_from(attribute).ok().filter(|&k| k < 4) {
                sums[k] = sums[k].wrapping_add(amount);
            }
        }
        for (debit, amount) in self.players[player].pending_debit.iter_mut().zip(sums) {
            *debit -= f32::from(amount);
        }
        let owner = &self.players[player];
        let state = &owner.techs[tech];
        let repeatable = state.repeatable(def, false);
        if def.kind == crate::tech::SHARED_KIND {
            return Err(Unsupported(
                "RESEARCH of the shared kind (FUN_1411c3fd0 over the packet's buildings)",
            ));
        }
        let building_id = self.least_loaded(player, r);
        if self.linked_techs.contains_key(&r.tech) {
            return Err(Unsupported(
                "RESEARCH of a linked technology (FUN_140f3a0f0, FUN_140f39e80)",
            ));
        }
        // a technology that is not available (state 1) and may not be queued again is
        // researchable only when it is linked (`FUN_140f39e80`): no building takes it
        if state.state != 1 && !repeatable {
            return Ok(());
        }
        let Some(i) = owner.objects.iter().position(|u| u.id == building_id) else {
            let elsewhere = self.players.iter().any(|q| {
                q.objects
                    .iter()
                    .chain(&q.sleeping)
                    .any(|u| u.id == building_id)
            });
            return if elsewhere {
                Err(Unsupported("RESEARCH at another player's object"))
            } else {
                Ok(())
            };
        };
        let building = &owner.objects[i];
        let master = self.master_of(player, building.life.master_id)?;
        let at = state
            .locations
            .iter()
            .any(|l| l.building == building.life.master_id);
        if master.kind != BUILDING_KIND || building.life.state != 2 || !at {
            return Ok(());
        }
        if owner.computer {
            return Err(Unsupported(
                "a computer player's RESEARCH (FUN_1411d5520, player+0x7a8)",
            ));
        }
        let top = building.top_action_kind().unwrap_or(0);
        if building.unmodelled_action.is_some() {
            return Err(Unsupported(
                "RESEARCH at a building with an unmodelled action",
            ));
        }
        if BLOCKS_RESEARCH.contains(&top) {
            return Ok(());
        }
        if r.tech == 0x66 {
            return Err(Unsupported(
                "RESEARCH of Castle Age (the game's +0x820 flag)",
            ));
        }
        if self.queue_research(player, i, r.tech, r.shift)? {
            let building = &mut self.players[player].objects[i];
            if let Some(ai) = building.ai.as_mut() {
                ai.set_action(RESEARCH_ACTION);
            }
        }
        Ok(())
    }

    /// `FUN_1411d70f0` for the building at `i` of `player`: one research, five with `shift`
    /// unless the technology allows only one at a time or its effect upgrades a unit, queued by
    /// the building's part with no master and no button ([`Turns::queue_at`]). True when any
    /// was queued. The game settings' instant technologies (`FUN_140f39f40`'s `+0x20`, which
    /// `FUN_1413cdcd0` researches at once) are linked ones, which RESEARCH refuses before.
    pub(in crate::turn) fn queue_research(
        &mut self,
        player: usize,
        i: usize,
        tech: i16,
        shift: bool,
    ) -> Result<bool, Unsupported> {
        let t = usize::try_from(tech).expect("RESEARCH queues a technology of the tree");
        let def = &self.techs[t];
        let state = &self.players[player].techs[t];
        let one = !state.allow_multiple || def.upgrades();
        let count: u16 = if !shift || one { 1 } else { 5 };
        self.queue_at(player, i, -1, count, tech, -1)
    }

    /// QUEUE, the player's part (`FUN_141614450`) after the packet's check of its buildings
    /// (`do_command`'s case 0x81, [`Turns::command_check`]: an id the world lacks, or another
    /// player's without cheats, goes; a human's packet left with none is refused). Nothing for
    /// a master the player lacks (`FUN_141659d20`). Of the first 60 buildings (500 for a computer
    /// player) those the master is available to (`+0x5f0`) or that list it themselves (`+0xd30`,
    /// `FUN_1411c8fd0`) take part, at most 60: each has its room (15, a computer player's 2, less
    /// its queued total `+0xbf4`; the sum of those above 0 holds the amount) and its load
    /// ([`Turns::production_load`]). The amount is lowered while the player cannot pay that many:
    /// each cost entry of the costed master ([`crate::production::costed`]; an annex's paid
    /// ones) but the population's needs its amount times the count (`COMISS`, `JA`: a NaN
    /// passes); at one still short, or for a master below type 70, nothing is queued. In game
    /// mode 16, while the player's attribute 6 is at most 1, a military master (its key's `+9`,
    /// [`crate::tracker::Key::military`]) is queued one at a time: with the player's flag
    /// (`+0x1378`) set nothing is, else the flag sets and the amount is 1 (even with no room).
    /// Then unit by unit the building with room and the least load takes it (the last one's
    /// pick first, replaced by one whose load is below: `COMISS`, `JBE`, a NaN never wins; with
    /// none at the start, the most room), its room one less and its load the unit's train time
    /// there ([`crate::production::train_time`]); last, each building's part
    /// ([`Turns::queue_at`]) with its count, the packet's master and button. Not modelled: a
    /// building that cannot train the unit (`FUN_141263610`, which reads the local player's
    /// masters), the limited units (`FUN_1416116c0`, `FUN_141611520`, the master's `+0x5f1`
    /// bits 2 and 4).
    ///
    /// # Errors
    ///
    /// A building of another player (with cheats) or asleep, and what the parts do not model.
    pub(in crate::turn) fn queue(&mut self, player: usize, q: &QueueAt) -> Result<(), Unsupported> {
        let Some(ids) = self.command_check(player, &q.buildings) else {
            return Ok(());
        };
        let owner = &self.players[player];
        let limit = if owner.computer { 500 } else { 60 };
        let empty = Masters::default();
        let masters = self.masters.get(player).unwrap_or(&empty);
        let Some(unit) = masters.get(q.unit) else {
            return Ok(());
        };
        let mut found = Vec::new();
        for place in self
            .lookup(&ids[..ids.len().min(limit)])
            .into_iter()
            .flatten()
        {
            let crate::order::Place::Live(p, i) = place else {
                return Err(Unsupported("QUEUE at a building the slice does not have"));
            };
            if p != player {
                return Err(Unsupported("QUEUE at a building the slice does not have"));
            }
            let listed = owner.objects[i]
                .production
                .as_ref()
                .is_some_and(|p| p.available_units.contains(&q.unit));
            if unit.available || listed {
                found.push(i);
            }
        }
        found.truncate(60);
        if found.is_empty() {
            return Ok(());
        }
        let per = if owner.computer {
            COMPUTER_QUEUE_LIMIT
        } else {
            QUEUE_LIMIT
        } as i32;
        let mut rooms = Vec::with_capacity(found.len());
        let mut loads = Vec::with_capacity(found.len());
        let mut total_room = 0i32;
        for &i in &found {
            let total = owner.objects[i]
                .production
                .as_ref()
                .map_or(0, |p| p.queue.total);
            let room = per - i32::from(total);
            total_room += room.max(0);
            rooms.push(room);
            loads.push(self.production_load(player, i));
        }
        let mut amount = i32::from(q.amount as i16).min(i32::from(total_room as i16));
        if 0 < amount {
            if unit.kind < crate::production::COMBAT_MASTER {
                return Ok(());
            }
            let (costed, annex) = crate::production::costed(masters, unit);
            let short = |count: i32| -> Result<bool, Unsupported> {
                for &(attribute, each, paid) in costed.cost.iter().take(3) {
                    if attribute < 0 || (annex && !paid) || attribute == POPULATION {
                        continue;
                    }
                    let held = crate::tech::Attributes::get(owner, attribute)?;
                    if held < f32::from(each) * count as f32 {
                        return Ok(true);
                    }
                }
                Ok(false)
            };
            while short(amount)? {
                if amount <= 1 {
                    return Ok(());
                }
                amount -= 1;
            }
        }
        // game mode 16: with attribute 6 at most 1 (COMISS, JC: a NaN is not) a military unit
        // (its key's +9) is queued one at a time, the player's +0x1378 its flag
        if self.settings.game_mode == ONE_UNIT_GAME
            && crate::tech::Attributes::get(owner, CURRENT_AGE)? <= ONE_UNIT_AGE
            && self.tracker_key(player, q.unit).military
        {
            if owner.one_military_queued {
                return Ok(());
            }
            self.players[player].one_military_queued = true;
            amount = 1;
        }
        let owner = &self.players[player];
        let mut counts = vec![0u16; found.len()];
        let mut pick: Option<usize> = None;
        while 0 < amount {
            amount -= 1;
            let mut best = pick;
            for k in 0..found.len() {
                if 0 < rooms[k] && best.is_none_or(|b| loads[b] > loads[k]) {
                    best = Some(k);
                }
            }
            if best.is_none() {
                for k in 0..found.len() {
                    if 0 < rooms[k] && best.is_none_or(|b| rooms[b] < rooms[k]) {
                        best = Some(k);
                    }
                }
            }
            let Some(k) = best else {
                return Ok(());
            };
            rooms[k] -= 1;
            counts[k] += 1;
            let at = owner.objects[found[k]].life.master_id;
            let time = crate::production::train_time(&unit.train_locations, at, q.button);
            loads[k] += f32::from(time);
            pick = Some(k);
        }
        for (k, &i) in found.iter().enumerate() {
            if 0 < counts[k] {
                let _ = self.queue_at(player, i, q.unit, counts[k], -1, q.button)?;
            }
        }
        Ok(())
    }

    /// RESEARCH's building (`0x140e7e270`): the packet's (`+4`, a building or none); with more
    /// than one listed (`+0x8` as a short; at most 60, 500 for a computer player,
    /// `FUN_140e6b250`), each listed one the world holds (`+0x11..`, `FUN_140e6bad0`) whose load
    /// ([`Turns::production_load`]) is below the one taken (`COMISS`, `JBE`: a NaN keeps it)
    /// takes its place. −1 for none.
    pub(in crate::turn) fn least_loaded(&self, player: usize, r: &ResearchAt) -> i32 {
        let load = |id: i32| match self.place(id) {
            Some(crate::order::Place::Live(q, i)) => self.production_load(q, i),
            _ => 0.0,
        };
        let is_building = |id: i32| {
            self.object(id).is_some_and(|(q, u)| {
                self.masters
                    .get(q)
                    .and_then(|m| m.get(u.life.master_id))
                    .is_some_and(|m| m.kind == BUILDING_KIND)
            })
        };
        let mut chosen = (is_building(r.building)).then_some(r.building);
        if 1 < r.count {
            let limit = if self.players[player].computer {
                500
            } else {
                60
            };
            let n = usize::from(r.count as u16).min(limit);
            for &id in r.buildings.iter().take(n) {
                if id == -1 || self.object(id).is_none() {
                    continue;
                }
                if chosen.is_none_or(|c| load(c) > load(id)) {
                    chosen = Some(id);
                }
            }
        }
        chosen.unwrap_or(-1)
    }

    /// `FUN_1411d0e90`: the seconds of work the building at `i` of player `p` has queued: each
    /// queue entry's count times its time (a unit's master's first train time, `+0x898`, in
    /// integers; a research's location at the building, `FUN_1413cbd20`, its time plus its
    /// adjustment, as floats), less the work done of its production list's action (a make's
    /// `+0x6c`, a research's `+0x70`).
    #[must_use]
    pub(crate) fn production_load(&self, p: usize, i: usize) -> f32 {
        let owner = &self.players[p];
        let building = &owner.objects[i];
        let Some(production) = building.production.as_ref() else {
            return 0.0;
        };
        let queue = &production.queue;
        let entries = usize::try_from(queue.count)
            .unwrap_or(0)
            .min(queue.slots.len());
        let mut queued = 0.0f32;
        for e in &queue.slots[..entries] {
            if e.master != -1 {
                let time = self
                    .masters
                    .get(p)
                    .and_then(|m| m.get(e.master))
                    .and_then(|m| m.train_locations.first())
                    .map_or(0, |l| l.time);
                queued += (i32::from(e.count) * i32::from(time)) as f32;
            } else if e.tech != -1 {
                // a queued research has its locations (the queue checked them)
                let (time, adjustment) = usize::try_from(e.tech)
                    .ok()
                    .and_then(|t| owner.techs.get(t))
                    .and_then(|t| t.location(building.life.master_id))
                    .map_or((0, 0.0), |l| (l.time as i16, l.adjustment));
                queued += (f32::from(time) + adjustment) * f32::from(e.count);
            }
        }
        if queue.slots.is_empty() {
            return queued;
        }
        let done = match production.action {
            Some(crate::production::Making::Unit(m)) => m.progress,
            Some(crate::production::Making::Research(r)) => r.progress,
            None => 0.0,
        };
        queued - (0.0 + done)
    }

    /// `FUN_1411cbc40`, QUEUE's and RESEARCH's building part, at the building at `i` of
    /// `player`: `count` of master `unit`, or with `tech` other than −1 that technology (`unit`
    /// −1), with the train button `button`. The generic villager (0x712) is the villager
    /// (0x53), whose sex the Action stream draws for each: the routine queues the rest first,
    /// one by one, then this one (the last drawn is queued first). A building whose total
    /// (`+0xbf4`) is above 14 refuses (its event 0x84 is the interface's); the count is held
    /// to the room of 15. Units: [`Turns::queue_units`]; researches: [`Turns::queue_techs`].
    /// True when any was queued.
    pub(in crate::turn) fn queue_at(
        &mut self,
        player: usize,
        i: usize,
        unit: i16,
        count: u16,
        tech: i16,
        button: i32,
    ) -> Result<bool, Unsupported> {
        let mut master = if unit == VILLAGER_ANY { VILLAGER } else { unit };
        let mut count = count;
        if master == VILLAGER {
            master = villager(self.streams.get(Stream::Action).raw64());
            if 0 < count.wrapping_sub(1) as i16 {
                let _ = self.queue_at(player, i, VILLAGER, count - 1, -1, button)?;
            }
            count = 1;
        }
        let total = self.players[player].objects[i]
            .production
            .as_ref()
            .map_or(0, |p| p.queue.total);
        if 14 < total {
            return Ok(false);
        }
        let n = if i32::from(count as i16) + i32::from(total) < 16 {
            count as i16
        } else {
            crate::production::ROOM - total
        };
        if tech == -1 {
            self.queue_units(player, i, master, n, button)
        } else {
            self.queue_techs(player, i, (master, tech), n, button)
        }
    }

    /// `FUN_1411cbc40`'s unit branch: `n` of `master` paid at once (`FUN_14162fb60`,
    /// [`crate::production::pay`]), the count lowered while the player cannot (at one, nothing
    /// is queued); attribute 80 (the queued units) grows by the master's population times the
    /// count (slot 0xf8); the player's tracker counts them and places each
    /// ([`crate::tracker::Tracker::place`]; `FUN_140a4f570` per unit is the local player's
    /// statistics). They join the last entry when it is of the same master and button and its
    /// costs are the ones a unit costs now (`FUN_1411c81a0`,
    /// [`crate::production::master_costs`]); else a new entry ([`Turns::new_entry`]) records
    /// them, with the master's train time at the building and button
    /// ([`crate::production::train_time`]) for a combat master.
    pub(in crate::turn) fn queue_units(
        &mut self,
        player: usize,
        i: usize,
        master: i16,
        count: i16,
        button: i32,
    ) -> Result<bool, Unsupported> {
        use crate::production::{QUEUED, as_entry_costs, master_costs, pay, population};
        let empty = Masters::default();
        let mut n = count;
        loop {
            let masters = self.masters.get(player).unwrap_or(&empty);
            if pay(masters, master, f32::from(n), &mut self.players[player])? {
                break;
            }
            if n < 2 {
                return Ok(false);
            }
            n -= 1;
        }
        let class = self.tracker_key(player, master).class;
        let masters = self.masters.get(player).unwrap_or(&empty);
        let made = masters.get(master);
        let pop = made.map_or(0.0, |m| population(&m.storage));
        let owner = &mut self.players[player];
        owner.add_attribute(QUEUED, pop * f32::from(n))?;
        let costs = as_entry_costs(master_costs(masters, master, &*owner)?);
        owner.tracker.count_queued(n);
        for _ in 0..n {
            owner.tracker.place(master, class);
        }
        let building = owner.objects[i].life.master_id;
        let production = owner.objects[i]
            .production
            .get_or_insert_with(Production::default);
        if let Some(last) = production.queue.last()
            && last.master == master
            && last.button == button
            && crate::production::same_costs(&last.costs, &costs)
        {
            let queue = &mut production.queue;
            let k = (queue.count - 1) as usize;
            queue.slots[k].count = queue.slots[k].count.wrapping_add(n);
            queue.total = queue.total.wrapping_add(n);
            queue.changes = queue.changes.wrapping_add(1);
            return Ok(true);
        }
        let time = made
            .filter(|m| crate::production::COMBAT_MASTER <= m.kind)
            .map(|m| crate::production::train_time(&m.train_locations, building, button));
        self.new_entry(player, i, (master, -1), n, button, time)?;
        Ok(true)
    }

    /// `FUN_1411cbc40`'s technology branch: the count is held to the technology's cap less its
    /// researches queued and done when it stacks (`+0x40`; the cap `+0x56`, 0x100 below 0; the
    /// player's `+0x648` is not held, 0 for every player probed); each research is paid
    /// (`FUN_14162fd90`: [`crate::tech::TechState::pay`] with the repeat test) until one cannot
    /// be. The paid ones join the last entry when it is of the same technology and its costs
    /// are the ones it lists now (`FUN_1411c81a0` -> `FUN_1413d4a00`,
    /// [`crate::tech::TechState::listed_costs`]); else a new entry ([`Turns::new_entry`]); the
    /// technology's researches queued (`+0x58`) grow by them.
    fn queue_techs(
        &mut self,
        player: usize,
        i: usize,
        (master, tech): (i16, i16),
        count: i16,
        button: i32,
    ) -> Result<bool, Unsupported> {
        let t = usize::try_from(tech).expect("RESEARCH queues a technology of the tree");
        let def = self
            .techs
            .get(t)
            .ok_or(Unsupported("a technology the data file has not got"))?;
        let owner = &mut self.players[player];
        let state = &owner.techs[t];
        let done = state.pending.wrapping_add(state.times_researched);
        let cap = if 0 <= state.research_cap {
            state.research_cap
        } else {
            0x100
        };
        let mut n = count;
        if state.stack && i32::from(cap) - i32::from(done) < i32::from(n) {
            n = cap.wrapping_sub(done);
        }
        // FUN_14162fd90's repeat flag: the player's +0x648 (not held) or FUN_1413d1050
        let repeat = state.repeatable(def, false);
        let building = owner.objects[i].life.master_id;
        let mut paid = 0i16;
        while paid < n {
            let get = |k: i16| crate::tech::Attributes::get(&*owner, k);
            let factors = crate::tech::factors(tech, &get)?;
            let mut state = std::mem::take(&mut owner.techs[t]);
            let ok = state.pay(def, factors, repeat, building, owner);
            owner.techs[t] = state;
            if !ok? {
                n = paid;
                break;
            }
            paid += 1;
        }
        if n <= 0 {
            return Ok(false);
        }
        let get = |k: i16| crate::tech::Attributes::get(&*owner, k);
        let listed = owner.techs[t].listed_costs(def, crate::tech::factors(tech, &get)?);
        let costs = crate::production::as_entry_costs(listed);
        let production = owner.objects[i]
            .production
            .get_or_insert_with(Production::default);
        if let Some(last) = production.queue.last()
            && last.tech == tech
            && crate::production::same_costs(&last.costs, &costs)
        {
            let queue = &mut production.queue;
            let k = (queue.count - 1) as usize;
            queue.slots[k].count = queue.slots[k].count.wrapping_add(n);
            queue.total = queue.total.wrapping_add(n);
            queue.changes = queue.changes.wrapping_add(1);
            owner.techs[t].pending = owner.techs[t].pending.wrapping_add(n);
            return Ok(true);
        }
        self.new_entry(player, i, (master, tech), n, button, None)?;
        let pending = &mut self.players[player].techs[t].pending;
        *pending = pending.wrapping_add(n);
        Ok(true)
    }

    /// `FUN_1411cbc40`'s new entry at the building at `i` of `player`: a full buffer grows by
    /// ten first, each old slot's costs filled anew ([`Turns::entry_costs`]); the slot after
    /// the last takes the master, the count, the technology, the button and the next serial,
    /// its costs [`Turns::entry_costs`], and the train time `time` when given (a research and a
    /// master that is not a combat master leave the slot's); the total, the entries and the
    /// change counter grow.
    fn new_entry(
        &mut self,
        player: usize,
        i: usize,
        (master, tech): (i16, i16),
        n: i16,
        button: i32,
        time: Option<i16>,
    ) -> Result<(), Unsupported> {
        let queue = &self.players[player].objects[i]
            .production
            .as_ref()
            .expect("the caller's production")
            .queue;
        let refill = if queue.full() {
            Some(
                queue
                    .slots
                    .iter()
                    .map(|s| self.entry_costs(player, s.tech, s.master))
                    .collect::<Result<Vec<_>, _>>()?,
            )
        } else {
            None
        };
        let costs = self.entry_costs(player, tech, master)?;
        let queue = &mut self.players[player].objects[i]
            .production
            .as_mut()
            .expect("the caller's production")
            .queue;
        if let Some(refill) = refill {
            queue.grow(&refill);
        }
        let k = queue.open(master, n, tech, button);
        queue.slots[k].costs = costs;
        if let Some(time) = time {
            queue.slots[k].train_time = time;
        }
        queue.total = queue.total.wrapping_add(n);
        queue.count += 1;
        queue.changes = queue.changes.wrapping_add(1);
        Ok(())
    }

    /// `FUN_1411c3190`: the costs a queue entry of `master` (for a master other than −1:
    /// [`crate::production::master_costs`]) or of technology `tech` records, from (−1, 0). A
    /// technology the tree has not got (`FUN_1413cc6c0`), or none, records none; one in state 2
    /// or 4 what [`crate::tech::TechState::entry_costs`] gives.
    pub(crate) fn entry_costs(
        &self,
        player: usize,
        tech: i16,
        master: i16,
    ) -> Result<[(i32, f32); 3], Unsupported> {
        use crate::production::{NO_COSTS, as_entry_costs, master_costs};
        let owner = &self.players[player];
        if master != -1 {
            let empty = Masters::default();
            let masters = self.masters.get(player).unwrap_or(&empty);
            return Ok(as_entry_costs(master_costs(masters, master, owner)?));
        }
        let Some(t) = usize::try_from(tech)
            .ok()
            .filter(|&t| t < owner.techs.len())
        else {
            return Ok(NO_COSTS);
        };
        let Some(def) = self.techs.get(t) else {
            return Ok(NO_COSTS);
        };
        let get = |k: i16| crate::tech::Attributes::get(owner, k);
        Ok(owner.techs[t].entry_costs(def, crate::tech::factors(tech, &get)?))
    }

    /// `FUN_140e6cfc0`: each of an order's units (INTERACT, MOVE, RETREAT) the world holds,
    /// sleeping ones too (`FUN_1415417a0`), with a unit AI resets it for the order
    /// ([`UnitAi::reset_for_order`]), in list order.
    pub(in crate::turn) fn reset_order_units(&mut self, units: &[i32]) {
        for &id in units {
            let Some(place) = self.place(id) else {
                continue;
            };
            let (p, i) = match place {
                crate::order::Place::Live(p, i) | crate::order::Place::Sleeping(p, i) => (p, i),
            };
            let player = &mut self.players[p];
            let unit = match place {
                crate::order::Place::Live(..) => &mut player.objects[i],
                crate::order::Place::Sleeping(..) => &mut player.sleeping[i],
            };
            if let Some(ai) = unit.ai.as_mut() {
                ai.reset_for_order(self.streams.get(Stream::Ai));
            }
        }
    }
}
