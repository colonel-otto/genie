//! The commands ([`Command`]) and their dispatch ([`World::run`]): the handlers of the
//! players' orders, the buildings' placement and the production queue's.
//!
//! [`World::run`]: crate::world::World::run

use crate::GenieError;
use crate::world::World;

pub mod all_back_to_work;
pub mod attack_move;
pub mod auto_scout;
pub mod build;
mod builders;
mod building_commands;
pub mod building_queue;
pub mod delete;
pub mod flare;
pub mod game;
pub mod gate;
pub mod gather_point;
pub mod go_back_to_work;
mod ground;
mod new_building;
pub mod order;
pub mod patrol;
mod placement;
mod production;
pub mod queue;
pub mod research;
pub mod resign;
mod run;
pub mod seek_shelter;
pub mod stop;
pub mod town_bell;
pub mod trade;
pub mod transform_object;
pub mod tribute;
pub mod unit_ai_state;
pub mod unit_order;
pub mod unload;
pub mod wall;

pub use self::all_back_to_work::AllBackToWorkCommand;
pub use self::attack_move::AttackMoveCommand;
pub use self::auto_scout::AutoScoutCommand;
pub use self::build::BuildCommand;
pub use self::building_queue::BuildingQueueCommand;
pub use self::delete::DeleteCommand;
pub use self::flare::FlareCommand;
pub use self::game::GameCommand;
pub use self::gate::GateCommand;
pub use self::gather_point::GatherPointCommand;
pub use self::go_back_to_work::GoBackToWorkCommand;
pub use self::order::{Order, OrderCommand, OrderUnits};
pub use self::patrol::PatrolCommand;
pub use self::queue::QueueCommand;
pub use self::research::ResearchCommand;
pub use self::resign::ResignCommand;
pub use self::seek_shelter::SeekShelterCommand;
pub use self::stop::StopCommand;
pub use self::town_bell::TownBellCommand;
pub use self::trade::TradeCommand;
pub use self::transform_object::TransformObjectCommand;
pub use self::tribute::TributeCommand;
pub use self::unit_ai_state::UnitAiStateCommand;
pub use self::unit_order::{UnitOrderAction, UnitOrderCommand};
pub use self::unload::UnloadCommand;
pub use self::wall::BuildWallCommand;

#[cfg(test)]
pub(crate) use self::production::least_loaded;
#[cfg(test)]
mod tests;

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

/// A recorded command, run in the command phase of its tick: after the frame seed's `SeedAll`,
/// before the players update (`TRIBE_Command::do_command`, `0x140e77b00`), each by its own run
/// ([`World::run`]). The slice runs only what other players' lanes see of a command; the issuing
/// player's lane stops.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    /// BUILD (0x66).
    Build(BuildCommand),
    /// `BUILD_WALL` (0x69).
    BuildWall(BuildWallCommand),
    /// SET GATHER POINT (0x78).
    GatherPoint(GatherPointCommand),
    /// QUEUE (0x81).
    Queue(QueueCommand),
    /// RESEARCH (0x65).
    Research(ResearchCommand),
    /// A command the slice runs part of or none, for a reason the decoder names: it stops the
    /// issuing player's lane.
    Unmodelled {
        /// The issuing player.
        player: usize,
        /// Why its command is not modelled.
        why: GenieError,
    },
    /// GAME (0x67).
    Game(GameCommand),
    /// `BUILDING_COMMAND_QUEUE` (0x2d).
    CommandQueue(BuildingQueueCommand),
    /// `GO_BACK_TO_WORK` (0x80, 0x86).
    GoBackToWork(GoBackToWorkCommand),
    /// `ALL_BACK_TO_WORK` (0x8c).
    AllBackToWork(AllBackToWorkCommand),
    /// `TOWN_BELL` (0x7f).
    TownBell(TownBellCommand),
    /// `CANCEL_BUILD` (0x6a).
    Delete(DeleteCommand),
    /// FLARE (0x73).
    Flare(FlareCommand),
    /// GIVE ATTRIBUTES (0xc4).
    Tribute(TributeCommand),
    /// `SELL_COMMODITY` (0x7a) and `BUY_COMMODITY` (0x7b).
    Trade(TradeCommand),
    /// GATE (0x72).
    Gate(GateCommand),
    /// UNLOAD (0x6f).
    Unload(UnloadCommand),
    /// `TRANSFORM_OBJECT` (0x29).
    TransformObject(TransformObjectCommand),
    /// RESIGN (0x0b).
    Resign(ResignCommand),
    /// `AUTO_SCOUT` (0x26).
    AutoScout(AutoScoutCommand),
    /// STOP (0x01).
    Stop(StopCommand),
    /// PATROL (0x15).
    Patrol(PatrolCommand),
    /// `ATTACK_MOVE` (0x21).
    AttackMove(AttackMoveCommand),
    /// `SEEK_SHELTER` (0x2c).
    SeekShelter(SeekShelterCommand),
    /// `UNIT_AI_STATE` (0x12).
    UnitAiState(UnitAiStateCommand),
    /// `UNIT_ORDER` (0x75).
    UnitOrder(UnitOrderCommand),
    /// INTERACT (0x00), WORK (0x02), MOVE (0x03), RETREAT (0x23), FOLLOW or GUARD.
    Order(OrderCommand),
}

impl Command {
    /// Its run on the world, by its kind: an unmodelled command fails with its reason.
    ///
    /// # Errors
    ///
    /// What the run does not model.
    pub(crate) fn run(&self, w: &mut World) -> Result<(), GenieError> {
        match self {
            Self::Build(c) => c.run(w),
            Self::BuildWall(c) => c.run(w),
            Self::GatherPoint(c) => c.run(w),
            Self::Queue(c) => c.run(w),
            Self::Research(c) => c.run(w),
            Self::Unmodelled { why, .. } => Err(why.clone()),
            Self::Game(c) => c.run(w),
            Self::CommandQueue(c) => c.run(w),
            Self::GoBackToWork(c) => c.run(w),
            Self::AllBackToWork(c) => c.run(w),
            Self::TownBell(c) => c.run(w),
            Self::Delete(c) => c.run(w),
            Self::Flare(c) => c.run(w),
            Self::Tribute(c) => c.run(w),
            Self::Trade(c) => {
                c.run(w);
                Ok(())
            }
            Self::Gate(c) => c.run(w),
            Self::Unload(c) => c.run(w),
            Self::TransformObject(c) => c.run(w),
            Self::Resign(c) => c.run(w),
            Self::AutoScout(c) => c.run(w),
            Self::Stop(c) => c.run(w),
            Self::Patrol(c) => c.run(w),
            Self::AttackMove(c) => c.run(w),
            Self::SeekShelter(c) => c.run(w),
            Self::UnitAiState(c) => c.run(w),
            Self::UnitOrder(c) => c.run(w),
            Self::Order(c) => c.run(w),
        }
    }

    /// The player whose lane a failed run stops, of `players` world players: a GAME
    /// sub-command's target, else its issuer ([`GameCommand::target`]); any other command's
    /// issuing player.
    #[must_use]
    pub fn stopped_player(&self, players: usize) -> usize {
        match self {
            Self::Game(g) => g.target(players).unwrap_or(g.issuer),
            other => other.player(),
        }
    }

    /// The issuing player (GAME's issuer).
    #[must_use]
    pub const fn player(&self) -> usize {
        match self {
            Self::Build(c) => c.player,
            Self::BuildWall(c) => c.player,
            Self::GatherPoint(c) => c.player,
            Self::Queue(c) => c.player,
            Self::Research(c) => c.player,
            Self::Unmodelled { player, .. } => *player,
            Self::Game(c) => c.issuer,
            Self::CommandQueue(c) => c.player,
            Self::GoBackToWork(c) => c.player,
            Self::AllBackToWork(c) => c.player,
            Self::TownBell(c) => c.player,
            Self::Delete(c) => c.player,
            Self::Flare(c) => c.player,
            Self::Tribute(c) => c.player,
            Self::Trade(c) => c.player,
            Self::Gate(c) => c.player,
            Self::Unload(c) => c.player,
            Self::TransformObject(c) => c.player,
            Self::Resign(c) => c.player,
            Self::AutoScout(c) => c.player,
            Self::Stop(c) => c.player,
            Self::Patrol(c) => c.player,
            Self::AttackMove(c) => c.player,
            Self::SeekShelter(c) => c.player,
            Self::UnitAiState(c) => c.player,
            Self::UnitOrder(c) => c.player,
            Self::Order(c) => c.player,
        }
    }
}

/// The player's attribute game mode 16's QUEUE reads (6, `+0x30`), and its bound
/// (`DAT_14330eb8c`).
const CURRENT_AGE: i16 = 6;
const ONE_UNIT_AGE: f32 = crate::math::image::turn::ONE_UNIT_AGE;

/// A build order on a foundation the world no longer holds.
pub(crate) const FOUNDATION_GONE: GenieError =
    GenieError::unsupported("a build order on a foundation that is gone");

/// Castle Age (`0x66`), whose RESEARCH marks the treaty monitor in [`CASTLE_AGE_MODE`].
const CASTLE_AGE: i16 = 0x66;
/// The game mode (`0x10`) in which a Castle Age queued marks the world's treaty monitor.
const CASTLE_AGE_MODE: i32 = 0x10;

/// The units a human player's command names at most (`FUN_140e6b290`'s `0x3c`).
const HUMAN_UNITS: usize = 60;
/// The units a computer player's command names at most (`FUN_141659cf0`, 500).
const COMPUTER_UNITS: usize = 500;

impl World {
    /// The command check's units (`FUN_140e553f0`, `FUN_140e55250`): each unit the world holds
    /// (`FUN_1415417a0`, live or sleeping) that is the player's own, or any while players may
    /// order each other's units ([`crate::world::settings::Settings::cheats_enabled`]); −1 for
    /// the others.
    pub(crate) fn checked_units(&self, player: usize, units: &[i32]) -> Vec<i32> {
        units
            .iter()
            .map(|&id| match self.place(id) {
                Some(p) if p.player() == player || self.settings.cheats_enabled => id,
                _ => -1,
            })
            .collect()
    }

    /// `FUN_140e6b290`: the units an order of comm player `player` names. Comm player 0 and
    /// one past the world's take the packet's list as it is; a count of `0xffff` there reads
    /// past the packet (no units here). Any other takes its first [`World::most_units`], or
    /// for a count of `0xffff` its last commanded units ([`World::last_commanded`], none before
    /// its first).
    pub(crate) fn command_units(&self, player: usize, units: &OrderUnits) -> Vec<i32> {
        let inside = player == 0 || self.players.len() <= player;
        match units {
            OrderUnits::Listed(ids) if inside => ids.clone(),
            OrderUnits::Listed(ids) => ids[..ids.len().min(self.most_units(player))].to_vec(),
            OrderUnits::LastCommanded if inside => Vec::new(),
            OrderUnits::LastCommanded => {
                self.last_commanded.get(player).cloned().unwrap_or_default()
            }
        }
    }

    /// The units a command of `player` names at most: 60, 500 for a computer
    /// (`FUN_141659cf0`); none for a player the world lacks.
    pub(crate) fn most_units(&self, player: usize) -> usize {
        match self.players.get(player) {
            Some(p) if p.computer => COMPUTER_UNITS,
            Some(_) => HUMAN_UNITS,
            None => 0,
        }
    }

    /// Comm player `player`'s last commanded units become `units` (`FUN_140e6d140` and the
    /// copy after it, `FUN_140e6b290` with 0); nothing for comm player 0 or one past the
    /// world's.
    pub(crate) fn set_last_commanded(&mut self, player: usize, units: Vec<i32>) {
        if player == 0 || self.players.len() <= player {
            return;
        }
        if self.last_commanded.len() <= player {
            self.last_commanded.resize(player + 1, Vec::new());
        }
        self.last_commanded[player] = units;
    }
}
