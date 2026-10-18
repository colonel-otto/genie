//! Provides an Age of Empires series recorded game file reader.
//!
//! ## Version Support
//! This crate can read Age of Empires 1, Age of Empires 2: The Conquerors, and HD Edition recorded game files.
//!
//! ## Credits
//! Most of the `.mgl`, `.mgx`, `.mgz` format specification was taken from Bari's classic [mgx
//! format description][], the [recage][] Node.js library, and Happyleaves' [aoc-mgz][] Python library.
//!
//! [mgx format description]: https://web.archive.org/web/20090215065209/http://members.at.infoseek.co.jp/aocai/mgx_format.html
//! [recage]: https://github.com/genie-js/recage
//! [aoc-mgz]: https://github.com/happyleavesaoc/aoc-mgz

#![deny(future_incompatible)]
#![deny(nonstandard_style)]
#![deny(rust_2018_idioms)]
#![deny(unsafe_code)]
#![warn(unused)]

pub mod actions;
pub mod ai;
pub mod de;
/// The trait shared by the header's parts.
pub mod element;
/// The recorded-game header (pre-DE layout).
pub mod header;
/// The map: tiles, zones, visibility and obstruction data.
pub mod map;
/// The players' state in the header.
pub mod player;
/// Readers that track the header's versions while reading.
pub mod reader;
/// The string table in the header.
pub mod string_table;
/// Units (game objects) in the header.
pub mod unit;
/// Unit actions (their current tasks) in the header.
pub mod unit_action;
/// Unit types (the per-player copies of the data file's units) in the header.
pub mod unit_type;
/// Game variant and version detection.
pub mod version;

use crate::actions::{Action, Meta};
use crate::element::ReadableHeaderElement;
use crate::reader::{RecordingHeaderReader, SmallBufReader};
use byteorder::{LE, ReadBytesExt};
use flate2::bufread::DeflateDecoder;
use genie_scx::DLCOptions;
use genie_support::{fallible_try_from, fallible_try_into, infallible_try_into};
pub use header::Header;
use std::fmt::Debug;
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom};
pub use version::*;

/// ID identifying a player (0-8).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PlayerID(u8);

impl PlayerID {
    /// Player ID for GAIA, the "nature" player.
    pub const GAIA: Self = Self(0);
}

impl From<u8> for PlayerID {
    #[inline]
    fn from(n: u8) -> Self {
        Self(n)
    }
}

impl From<PlayerID> for u8 {
    #[inline]
    fn from(player_id: PlayerID) -> Self {
        player_id.0
    }
}

fallible_try_from!(PlayerID, i32);
fallible_try_from!(PlayerID, u32);
fallible_try_from!(PlayerID, i16);
fallible_try_from!(PlayerID, u16);
fallible_try_from!(PlayerID, i8);
infallible_try_into!(PlayerID, i16);
infallible_try_into!(PlayerID, u16);
infallible_try_into!(PlayerID, i32);
infallible_try_into!(PlayerID, u32);

/// ID identifying a game object (unit, building, resource …).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ObjectID(u32);

impl From<u32> for ObjectID {
    #[inline]
    fn from(n: u32) -> Self {
        Self(n)
    }
}

impl From<u16> for ObjectID {
    #[inline]
    fn from(n: u16) -> Self {
        Self(n.into())
    }
}

impl From<ObjectID> for u32 {
    #[inline]
    fn from(n: ObjectID) -> Self {
        n.0
    }
}

fallible_try_from!(ObjectID, i16);
fallible_try_from!(ObjectID, i32);
fallible_try_into!(ObjectID, i16);
fallible_try_into!(ObjectID, i32);

/// The body's sync (checksum) actions did not arrive at the interval the body's meta action set.
#[derive(Debug, Clone, Copy, thiserror::Error)]
pub enum SyncError {
    /// A sync action arrived this many time actions too early.
    #[error(
        "Got a sync message, but the log header said there would be a sync message {0} ticks later. The recorded game file may be corrupt"
    )]
    UnexpectedSync(u32),
    /// A time action arrived where a sync action was due.
    #[error("Expected a sync message at this point, the recorded game file may be corrupt")]
    ExpectedSync,
}

/// Errors that may occur while reading a recorded game file.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Reading the file failed.
    #[error(transparent)]
    IoError(#[from] io::Error),
    /// The body's sync actions are out of step.
    #[error(transparent)]
    SyncError(#[from] SyncError),
    /// A string could not be decoded.
    #[error(transparent)]
    DecodeStringError(#[from] genie_support::DecodeStringError),
    /// The scenario embedded in the header could not be read.
    #[error("Could not read embedded scenario data: {0}")]
    ReadScenarioError(#[from] genie_scx::Error),
}

impl From<genie_support::ReadStringError> for Error {
    fn from(err: genie_support::ReadStringError) -> Self {
        match err {
            genie_support::ReadStringError::DecodeStringError(inner) => inner.into(),
            genie_support::ReadStringError::IoError(inner) => inner.into(),
        }
    }
}

/// Result type alias with `genie_rec::Error` as the error type.
pub type Result<T> = std::result::Result<T, Error>;

/// Iterator over body actions.
#[derive(Debug)]
pub struct BodyActions<R>
where
    R: BufRead,
{
    input: R,
    meta: Meta,
    remaining_syncs_until_checksum: u32,
}

impl<R> BodyActions<R>
where
    R: BufRead,
{
    /// Read the body's meta action and start iterating over the actions after it.
    ///
    /// `data_version` is the header's save version: from 11.76 on, the meta action has the mgx
    /// layout, before it the mgl one.
    ///
    /// # Errors
    ///
    /// Returns an error if the meta action cannot be read.
    pub fn new(mut input: R, data_version: f32) -> Result<Self> {
        let meta = if data_version >= 11.76 {
            Meta::read_from_mgx(&mut input)?
        } else {
            Meta::read_from_mgl(&mut input)?
        };
        let remaining_syncs_until_checksum = meta.checksum_interval;
        Ok(Self {
            input,
            meta,
            remaining_syncs_until_checksum,
        })
    }
}

impl<R> Iterator for BodyActions<R>
where
    R: BufRead,
{
    type Item = Result<Action>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.meta.use_sequence_numbers {
            let _sequence = match self.input.read_u8() {
                Ok(s) => s,
                Err(err) => return Some(Err(err.into())),
            };
        }
        match self.input.read_i32::<LE>() {
            Ok(0x00) => {
                if self.remaining_syncs_until_checksum == 0 {
                    self.remaining_syncs_until_checksum = self.meta.checksum_interval;
                    Some(actions::Sync::read_from(&mut self.input).map(Action::Sync))
                } else {
                    Some(Err(SyncError::UnexpectedSync(
                        self.remaining_syncs_until_checksum,
                    )
                    .into()))
                }
            }
            Ok(0x01) => Some(actions::Command::read_from(&mut self.input).map(Action::Command)),
            Ok(0x02) => {
                match self.remaining_syncs_until_checksum.checked_sub(1) {
                    Some(n) => self.remaining_syncs_until_checksum = n,
                    None => return Some(Err(SyncError::ExpectedSync.into())),
                }
                Some(actions::Time::read_from(&mut self.input).map(Action::Time))
            }
            Ok(0x03) => Some(actions::ViewLock::read_from(&mut self.input).map(Action::ViewLock)),
            Ok(0x04) => Some(actions::Chat::read_from(&mut self.input).map(Action::Chat)),
            Ok(id) => panic!("unsupported action type {id:#x}"),
            Err(err) if err.kind() == io::ErrorKind::UnexpectedEof => None,
            Err(err) => Some(Err(err.into())),
        }
    }
}

/// The AI difficulty setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Difficulty {
    /// The easiest difficulty.
    Easiest,
    /// Easy.
    Easy,
    /// Standard (moderate).
    Standard,
    /// Hard.
    Hard,
    /// Hardest.
    Hardest,
    /// Age of Empires 2: Definitive Edition only.
    Extreme,
}

/// The map size setting. Placeholder: no variants are modelled yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MapSize {}

/// The map type (random map script) setting. Placeholder: no variants are modelled yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MapType {}

/// The map visibility setting (normal, explored, all visible). Placeholder: no variants are
/// modelled yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Visibility {}

/// The starting resources setting. Placeholder: no variants are modelled yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ResourceLevel {}

/// An age (Dark, Feudal, Castle, Imperial, Post-Imperial). Placeholder: no variants are modelled
/// yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Age {}

/// The game mode setting (random map, regicide, death match …). Placeholder: no variants are
/// modelled yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GameMode {}

/// The game speed setting. Placeholder: no variants are modelled yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GameSpeed {}

/// The game settings an HD Edition recording stores. Not read by this crate yet.
// The bools mirror the settings' on/off flags.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone)]
pub struct HDGameOptions {
    /// The DLCs the game uses.
    pub dlc_options: DLCOptions,
    /// The AI difficulty.
    pub difficulty: Difficulty,
    /// The map size.
    pub map_size: MapSize,
    /// The map type.
    pub map_type: MapType,
    /// The map visibility.
    pub visibility: Visibility,
    /// The starting resources.
    pub starting_resources: ResourceLevel,
    /// The age the players start in.
    pub starting_age: Age,
    /// The last age the players can reach.
    pub ending_age: Age,
    /// The game mode.
    pub game_mode: GameMode,
    // if version < 1001
    /// The random map script's name.
    pub random_map_name: Option<String>,
    // if version < 1001
    /// The scenario's name.
    pub scenario_name: Option<String>,
    /// The game speed.
    pub game_speed: GameSpeed,
    /// The treaty length (time before players may attack each other).
    pub treaty_length: i32,
    /// The population limit.
    pub population_limit: i32,
    /// The number of players.
    pub num_players: i32,
    /// The amount the victory condition needs (score, time …).
    pub victory_amount: i32,
    /// Whether trading is allowed.
    pub trading_enabled: bool,
    /// Whether team bonuses are on.
    pub team_bonuses_enabled: bool,
    /// Whether the starting positions are randomized.
    pub randomize_positions_enabled: bool,
    /// Whether every civilization gets the full tech tree.
    pub full_tech_tree_enabled: bool,
    /// The number of units each player starts with.
    pub num_starting_units: i8,
    /// Whether the teams are locked.
    pub teams_locked: bool,
    /// Whether the game speed is locked.
    pub speed_locked: bool,
    /// Whether it is a multiplayer game.
    pub multiplayer: bool,
    /// Whether cheats are allowed.
    pub cheats_enabled: bool,
    /// Whether the game is recorded.
    pub record_game: bool,
    /// Whether (huntable) animals are on the map.
    pub animals_enabled: bool,
    /// Whether predators are on the map.
    pub predators_enabled: bool,
    // if version > 1.16 && version < 1002
    /// The scenario player each player plays.
    pub scenario_player_indices: Vec<i32>,
}

/// Recorded game reader.
#[derive(Debug)]
pub struct RecordedGame<R>
where
    R: Read + Seek,
{
    inner: R,
    /// Offset of the main compressed header.
    header_start: u64,
    /// Size of the compressed header.
    header_end: u64,
    /// Offset of the next header, for saved chapters.
    #[allow(unused)]
    next_header: Option<u64>,
    #[allow(unused)]
    game_version: GameVersion,
    save_version: f32,
}

impl<R> RecordedGame<R>
where
    R: Read + Seek,
{
    /// Read the file's offsets and versions; the header and body are read on demand.
    ///
    /// # Errors
    ///
    /// Returns an error if seeking or reading the first bytes fails.
    pub fn new(mut input: R) -> Result<Self> {
        let file_size = {
            let size = input.seek(SeekFrom::End(0))?;
            // Rewinding to the start: the new position is known.
            let _ = input.seek(SeekFrom::Start(0))?;
            size
        };

        let header_end = u64::from(input.read_u32::<LE>()?);
        let next_header = u64::from(input.read_u32::<LE>()?);

        let header_start = if next_header > file_size { 4 } else { 8 };

        let next_header = if next_header > 0 && next_header < file_size {
            Some(next_header)
        } else {
            None
        };

        let (game_version, save_version) = {
            let _ = input.seek(SeekFrom::Start(header_start))?;
            let version_reader = SmallBufReader::new(&mut input);
            let mut deflate = DeflateDecoder::new(version_reader);
            let game_version = GameVersion::read_from(&mut deflate)?;
            let save_version = deflate.read_f32::<LE>()?;
            (game_version, save_version)
        };

        Ok(Self {
            inner: input,
            header_start,
            header_end,
            next_header,
            game_version,
            save_version,
        })
    }

    fn seek_to_first_header(&mut self) -> Result<()> {
        let _ = self.inner.seek(SeekFrom::Start(self.header_start))?;

        Ok(())
    }

    fn seek_to_body(&mut self) -> Result<()> {
        let _ = self.inner.seek(SeekFrom::Start(self.header_end))?;

        Ok(())
    }

    /// Inflate and read the header.
    ///
    /// # Errors
    ///
    /// Returns an error if the header cannot be read or decoded.
    pub fn header(&mut self) -> Result<Header> {
        self.seek_to_first_header()?;
        let reader = BufReader::new(&mut self.inner).take(self.header_end - self.header_start);
        let deflate = DeflateDecoder::new(reader);
        let header = Header::read_from(&mut RecordingHeaderReader::new(deflate))?;
        Ok(header)
    }

    /// Iterate over the body's actions.
    ///
    /// # Errors
    ///
    /// Returns an error if seeking to the body or reading its meta action fails.
    pub fn actions(&mut self) -> Result<BodyActions<BufReader<&mut R>>> {
        self.seek_to_body()?;
        BodyActions::new(BufReader::new(&mut self.inner), self.save_version)
    }

    /// Give back the underlying reader.
    pub fn into_inner(self) -> R {
        self.inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;

    #[test]
    // AI data parsing is incomplete: remove this attribute when the test starts passing
    #[should_panic = "assertion"] // matches the pre- and post-1.73 assert_eq! panic text
    fn incomplete_up_15_rec_with_ai() {
        let f = File::open("test/rec.20181208-195117.mgz").unwrap();
        let mut r = RecordedGame::new(f).unwrap();
        let _ = r.header().expect("AI data cannot be fully parsed");
        for act in r.actions().unwrap() {
            let _act = act.unwrap();
        }
    }

    #[test]
    fn aoc_1_0_rec() -> anyhow::Result<()> {
        let f = File::open("test/missyou_finally_vs_11.mgx")?;
        let mut r = RecordedGame::new(f)?;
        let _ = r.header()?;
        for act in r.actions()? {
            match act {
                Ok(act) => println!("{act:?}"),
                Err(Error::DecodeStringError(_)) => {
                    // Skip invalid utf8 chat for now
                }
                Err(err) => return Err(err.into()),
            }
        }
        Ok(())
    }

    #[test]
    fn aok_rec() -> anyhow::Result<()> {
        let f = File::open("test/aok.mgl")?;
        let mut r = RecordedGame::new(f)?;
        let _ = r.header()?;
        for act in r.actions()? {
            println!("{:?}", act?);
        }
        Ok(())
    }
}
