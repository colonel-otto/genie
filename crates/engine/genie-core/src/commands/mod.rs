//! The commands ([`Command`]) and their dispatch ([`Turns::run`]): the handlers of the
//! players' orders, the buildings' placement and the production queue's.
//!
//! [`Turns::run`]: crate::world::Turns::run

use crate::orders::{Interact, MoveOrder};
use crate::unit_ai::GenieError;

mod builders;
mod building_commands;
pub mod delete;
pub(crate) mod game;
mod ground;
mod new_building;
mod placement;
mod production;
mod run;
pub mod wall;

#[cfg(test)]
pub(crate) use self::production::least_loaded;

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
        unit_ai: Result<Option<u32>, GenieError>,
        /// Where it is placed.
        position: [f32; 3],
        /// The units told to build it, in packet order.
        builders: Vec<i32>,
        /// `+0x1e`: queued (a builder with a unit AI takes it as an order in its queue).
        queued: bool,
        /// `+0x1f` is 0: a builder with a unit AI holding an order not ready takes it as an order
        /// too, and the queue's orders become ready after (`FUN_140c42b60`).
        commits: bool,
        /// `+0x1c`: the frame the foundation's standing sprite takes once built (its `+0xc54`,
        /// [`crate::world::Unit::build_frame`]).
        frame: u8,
    },
    /// `BUILD_WALL` (0x69, `FUN_140e7b2e0`): a line of the player's wall foundations and its
    /// builders shared out over them ([`crate::commands::wall`]).
    BuildWall(wall::Wall),
    /// SET GATHER POINT (0x78, `FUN_140e7e9a0`): each of the player's objects of `ids` that takes
    /// a gather point (its master garrisons, `+0x5d4`, or its player trains a unit there,
    /// `FUN_141625a20`'s count with no object, [`Turns::trainable_count`]) takes `point` and
    /// `target` with its master, or with `own` itself (`FUN_1411c7ff0`); a target of −1 at
    /// (−1, −1) clears it, and its unit AI's command queue empties (slot 0x210). The ids of
    /// other players' objects are left out (the game setting `+0x1074` that lets them is not
    /// modelled), as is the exists byte's value 0xff (`+0xc30`, which keeps the target).
    ///
    /// [`Turns::trainable_count`]: crate::world::Turns::trainable_count
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
    ///
    /// [`Turns::queue`]: crate::world::Turns::queue
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
    ///
    /// [`Turns::research`]: crate::world::Turns::research
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
        why: GenieError,
    },
    /// GAME (0x67): `do_command`'s case and its switch on the sub-command (`0x140e7c330`,
    /// [`Turns::game`]).
    ///
    /// [`Turns::game`]: crate::world::Turns::game
    Game(crate::world::GameCommand),
    /// `BUILDING_COMMAND_QUEUE` (0x2d, `0x140e64310`): the objects `ids` (at most a human
    /// player's 60) queue a step to `target` (−1 for none) or `point` ([`Turns::command_queue`]).
    ///
    /// [`Turns::command_queue`]: crate::world::Turns::command_queue
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
    ///
    /// [`Turns::go_back_to_work`]: crate::world::Turns::go_back_to_work
    GoBackToWork {
        /// The player.
        player: usize,
        /// Its buildings.
        ids: Vec<i32>,
    },
    /// `ALL_BACK_TO_WORK` (0x8c, the object at `+4`): its owner's buildings near it let their
    /// villagers out to work and the others go back ([`Turns::all_back_to_work`]).
    ///
    /// [`Turns::all_back_to_work`]: crate::world::Turns::all_back_to_work
    AllBackToWork {
        /// The player.
        player: usize,
        /// The object (a town centre).
        id: i32,
    },
    /// `TOWN_BELL` (0x7f, `do_command` case 0x7f): the town centre `id` rings its owner's bell
    /// (`activate`) or ends it ([`Turns::town_bell`]).
    ///
    /// [`Turns::town_bell`]: crate::world::Turns::town_bell
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
    ///
    /// [`Turns::delete`]: crate::world::Turns::delete
    Delete {
        /// The player.
        player: usize,
        /// The object.
        id: i32,
    },
    /// FLARE (0x73, `0x140e7bfd0`): every player from 1 below `players` (`+0x10`, at most the
    /// world's) gets a flare at `target`'s place (`+4`, −1 for none) or at `point` (`+8`, `+0xc`)
    /// ([`Turns::flare`]).
    ///
    /// [`Turns::flare`]: crate::world::Turns::flare
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
    ///
    /// [`Turns::tribute`]: crate::world::Turns::tribute
    Tribute {
        /// The player who gives.
        player: usize,
        /// The player who receives (`+0x2c`).
        to: usize,
        /// The four entries.
        gifts: [crate::players::tribute::Gift; 4],
    },
    /// `SELL_COMMODITY` (0x7a) and `BUY_COMMODITY` (0x7b), `do_command`'s cases: the player trades
    /// `times` lots (`+0x6`) of the commodity `attribute` (`+0x4`'s low byte: food, wood or
    /// stone; the decoder drops another) at the market ([`Turns::sell`], [`Turns::buy`]). The
    /// market building (`+0x8`) is not read. Not held: the interface's view (`FUN_140e6de20`).
    ///
    /// [`Turns::buy`]: crate::world::Turns::buy
    /// [`Turns::sell`]: crate::world::Turns::sell
    Trade {
        /// The player who trades.
        player: usize,
        /// The commodity.
        attribute: u8,
        /// How many lots.
        times: u16,
        /// A purchase, else a sale.
        buy: bool,
    },
    /// GATE (0x72, `do_command` case 0x72): the player's building `id` toggles its gate lock
    /// ([`Turns::gate`]).
    ///
    /// [`Turns::gate`]: crate::world::Turns::gate
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
    ///
    /// [`Turns::unload`]: crate::world::Turns::unload
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
    ///
    /// [`Turns::transform_object`]: crate::world::Turns::transform_object
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
    ///
    /// [`Turns::resign`]: crate::world::Turns::resign
    Resign {
        /// The comm player (the world player of the same number in the slice's games).
        player: usize,
        /// A drop from the game, not a resignation.
        dropped: bool,
    },
    /// `AUTO_SCOUT` (0x26, `FUN_140e666f0`): each of `ids` (at most 60, the decoder's) takes the
    /// auto scout order, or the stop order when it scouts already ([`Turns::auto_scout`]).
    ///
    /// [`Turns::auto_scout`]: crate::world::Turns::auto_scout
    AutoScout {
        /// The player.
        player: usize,
        /// Its units.
        ids: Vec<i32>,
    },
    /// STOP (0x01, `FUN_14161e200`): the units `ids` (at most 60, the decoder's) stop
    /// ([`Turns::stop_command`]).
    ///
    /// [`Turns::stop_command`]: crate::world::Turns::stop_command
    Stop {
        /// The player whose units stop.
        player: usize,
        /// The units.
        ids: Vec<i32>,
    },
    /// PATROL (0x15, `FUN_140e69cc0`): the units `ids` (the packet's count at `+0x4`, the ids
    /// from `+0x5c`) patrol through `waypoints` (1 to 10, their count at `+0xb`, x from `+0xc`, y
    /// from `+0x34`); `reset` (`+0x8`) drops their unit AIs' saved patrols ([`Turns::patrol`]).
    ///
    /// [`Turns::patrol`]: crate::world::Turns::patrol
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
    /// `ATTACK_MOVE` (0x21, `FUN_140e66200`): the units `ids` (the packet's count at `+0x4`, the
    /// ids from `+0x5c`) attack-move through `waypoints` (1 to 10, their count at `+0xb`, x from
    /// `+0xc`, y from `+0x34`); `reset` (`+0x8`) drops their unit AIs' saved patrols, `queued`
    /// (`+0x9`) and `ignore_pending` (`+0xa`) send the order through the unit AIs' queues
    /// ([`Turns::attack_move`]).
    ///
    /// [`Turns::attack_move`]: crate::world::Turns::attack_move
    AttackMove {
        /// The player whose units attack-move.
        player: usize,
        /// The units.
        ids: Vec<i32>,
        /// The waypoints.
        waypoints: Vec<[f32; 2]>,
        /// `+0x8`.
        reset: bool,
        /// `+0x9`.
        queued: bool,
        /// `+0xa`.
        ignore_pending: bool,
    },
    /// `SEEK_SHELTER` (0x2c, `FUN_140e6a3a0`): the villagers of `ids` (at most 60, the decoder's)
    /// seek shelter in the player's buildings ([`Turns::seek_shelter`]).
    ///
    /// [`Turns::seek_shelter`]: crate::world::Turns::seek_shelter
    SeekShelter {
        /// The player whose units seek shelter.
        player: usize,
        /// The units.
        ids: Vec<i32>,
    },
    /// `UNIT_AI_STATE` (0x12, `FUN_140e6a7c0`): the unit AIs of `ids` (at most 60, the decoder's)
    /// take the stance `state` ([`Turns::unit_ai_state`]).
    ///
    /// [`Turns::unit_ai_state`]: crate::world::Turns::unit_ai_state
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
    ///
    /// [`Turns::unit_order`]: crate::world::Turns::unit_order
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
    ///
    /// [`UnitAi::reset_for_order`]: crate::unit_ai::UnitAi::reset_for_order
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
    /// GUARD's target (`+0x8`).
    Guard(i32),
}

/// RESEARCH's fields ([`Command::Research`] but the player).
pub(crate) struct ResearchAt {
    pub(crate) building: i32,
    pub(crate) count: i16,
    pub(crate) buildings: Vec<i32>,
    pub(crate) tech: i16,
    pub(crate) target: i32,
    pub(crate) shift: bool,
}

/// QUEUE's fields ([`Command::Queue`] but the player).
pub(crate) struct QueueAt {
    pub(crate) buildings: Vec<i32>,
    pub(crate) unit: i16,
    pub(crate) amount: u16,
    pub(crate) button: i32,
}

/// The player's attribute game mode 16's QUEUE reads (6, `+0x30`), and its bound
/// (`DAT_14330eb8c`).
const CURRENT_AGE: i16 = 6;
const ONE_UNIT_AGE: f32 = crate::math::image::turn::ONE_UNIT_AGE;

/// A build order on a foundation the world no longer holds.
pub(crate) const FOUNDATION_GONE: GenieError =
    GenieError::unsupported("a build order on a foundation that is gone");
/// A build order whose unit has no task for its foundation (slot 0x380's 0).
pub(crate) const NO_BUILD_TASK: GenieError =
    GenieError::unsupported("a build order with no task for its foundation");

/// Castle Age (`0x66`), whose RESEARCH marks the treaty monitor in [`CASTLE_AGE_MODE`].
const CASTLE_AGE: i16 = 0x66;
/// The game mode (`0x10`) in which a Castle Age queued marks the world's treaty monitor.
const CASTLE_AGE_MODE: i32 = 0x10;
