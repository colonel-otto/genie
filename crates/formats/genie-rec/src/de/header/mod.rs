//! The world save at the start of a chapter, read as far as each player's object lists.
//!
//! Hand-ported from aoe2rec's `patterns/aoe2header.hexpat` (MIT, see `NOTICE`) and checked byte
//! for byte against ImHex's offsets for that pattern. Read-only: the writer will be generated from
//! the codegen IR that replaces this port (workspace FD-006-D05). Version gates compare
//! [`SaveVersion::hundredths`](super::SaveVersion::hundredths): 6660 is save 66.6.

mod ai;
mod lobby;
mod main_object;
mod map;
mod player;
mod primitives;
mod replay;

pub use ai::{AiConfig, AiData, AiRule, AiRuleGroup, SizedString32};
pub use lobby::{AiFile, LobbyPlayer, LobbySettings};
pub use main_object::MainObject;
pub use map::{MapInfo, MapZone, Obstructions, Tile};
pub use player::{
    InitialMap, PlayerEnds, PlayerPrelude, PlayerSection, RestoreObject, StateRestore, TechUnlock,
    UnitUnlock, Unlock, Unlocks,
};
pub use primitives::{DeString, StringList};
pub use replay::Replay;

use binrw::BinRead;

/// The fixed fields before the lobby settings.
#[derive(BinRead, Debug, Clone, PartialEq)]
#[br(little)]
pub struct VersionBlock {
    #[br(magic = b"VER 9.4\0")]
    pub old_save_version: f32,
    pub minor: u16,
    pub major: u16,
    pub build: u32,
    pub timestamp: i32,
    pub version: [u16; 2],
    pub interval_version: [u16; 2],
}

impl VersionBlock {
    /// The save version in hundredths, e.g. 6660 for 66.6.
    pub fn hundredths(&self) -> u32 {
        super::SaveVersion {
            major: self.major,
            minor: self.minor,
            build: self.build,
        }
        .hundredths()
    }
}

/// The oldest save version this reader supports (66.0).
pub const MIN_HUNDREDTHS: u32 = 6600;

/// Where the sections of the header end, for checks against other readers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SectionEnds {
    pub lobby: usize,
    pub ai: usize,
    pub replay: usize,
    pub map: usize,
    /// The first player section starts here.
    pub players_start: usize,
}

/// The header, as far as it is read so far.
#[derive(Debug, Clone, PartialEq)]
pub struct Header {
    pub version: VersionBlock,
    pub lobby: LobbySettings,
    pub ai: AiConfig,
    pub replay: Replay,
    pub map: MapInfo,
    pub restore_time: u32,
    /// `particles_count` records of 27 bytes.
    pub particles: Vec<u8>,
    pub identifier: u32,
    pub ends: SectionEnds,
    /// One section per player, Gaia first.
    pub players: Vec<PlayerSection>,
}

/// Read an inflated chapter header.
pub fn parse(inflated: &[u8]) -> Result<Header, super::Error> {
    let mut r = binrw::io::Cursor::new(inflated);
    let version = VersionBlock::read(&mut r).map_err(header_error)?;
    let v = version.hundredths();
    if v < MIN_HUNDREDTHS {
        return Err(super::Error::Header {
            message: format!("save version {v} is older than 66.0"),
        });
    }
    let lobby = LobbySettings::read_args(&mut r, (v,)).map_err(header_error)?;
    let mut ends = SectionEnds {
        lobby: r.position() as usize,
        ..SectionEnds::default()
    };
    // aoe2rec's AI layout fails on current single-player games with AI scripts, in ImHex too.
    let ai_start = r.position() as usize;
    let ai = AiConfig::read(&mut r).map_err(|e| match inflated.get(ai_start..ai_start + 4) {
        Some([1, 0, 0, 0]) => super::Error::Unsupported {
            what: "AI data (has_ai = 1)",
            at: ai_start,
        },
        _ => header_error(e),
    })?;
    ends.ai = r.position() as usize;
    let replay = Replay::read(&mut r).map_err(header_error)?;
    ends.replay = r.position() as usize;
    let map = MapInfo::read(&mut r).map_err(header_error)?;
    ends.map = r.position() as usize;
    let restore_time = u32::read_le(&mut r).map_err(header_error)?;
    let particles_count = u32::read_le(&mut r).map_err(header_error)?;
    let particles = Vec::<u8>::read_le_args(
        &mut r,
        binrw::VecArgs::builder()
            .count(particles_count as usize * 27)
            .finalize(),
    )
    .map_err(header_error)?;
    let identifier = u32::read_le(&mut r).map_err(header_error)?;
    ends.players_start = r.position() as usize;
    // Step C: the first player's section up to its object lists.
    let num_players = replay.num_players as usize;
    let players = vec![
        player::read_player(&mut r, v, num_players).map_err(|e| match e {
            player::ReadError::Bin(e) => header_error(e),
            player::ReadError::Unsupported { what, at } => super::Error::Unsupported { what, at },
        })?,
    ];
    Ok(Header {
        version,
        lobby,
        ai,
        replay,
        map,
        restore_time,
        particles,
        identifier,
        ends,
        players,
    })
}

fn header_error(e: binrw::Error) -> super::Error {
    super::Error::Header {
        message: e.to_string(),
    }
}
