//! The world save at the start of a chapter, read sequentially through every player section.
//!
//! Hand-ported from aoe2rec's `patterns/aoe2header.hexpat` (MIT, see `NOTICE`) and checked byte
//! for byte against ImHex's offsets for that pattern. Where the pattern guesses (fixed sizes,
//! look-aheads, delimiter runs, the search for the next player), the reader follows the save
//! functions of build 158041 instead; each such part names its function. Read-only: the writer
//! will be generated from the codegen IR that replaces this port (workspace FD-006-D05). Version
//! gates compare [`SaveVersion::hundredths`](super::SaveVersion::hundredths): 6660 is save 66.6.

mod ai;
pub mod generated;
mod layout_types;
mod lobby;
mod map;
mod objects;
mod player;
mod primitives;
mod replay;
#[cfg(test)]
mod test_bytes;

pub use ai::{AiConfig, AiData, AiRule, AiRuleGroup, SizedString32};
pub use generated::player::{Object, ObjectList, Player, PlayerObjects, WorldPlayer};
pub use layout_types::{LenString16, LenString32, LenVec32, STRING_SIGNATURE, SigString};
pub use layout_types::{Unsupported, UnsupportedFeature, unsupported_in};
pub use lobby::{AiFile, LobbyPlayer, LobbySettings};
pub use map::{MapInfo, MapZone, Obstructions, Tile};
pub use player::{PlayerEnds, PlayerSection};
pub use primitives::{DeString, StringList};
pub use replay::Replay;

use binrw::BinRead;
use binrw::io::{Read, Seek, SeekFrom};

/// The fixed fields before the lobby settings.
#[derive(BinRead, binrw::BinWrite, Debug, Clone, PartialEq)]
#[brw(little)]
pub struct VersionBlock {
    #[brw(magic = b"VER 9.4\0")]
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

    /// The version as the game compares it: 16.16 fixed point, e.g. `0x429999` for 66.6 (158041
    /// `DAT_143cd8948`). The generated readers' version gates take this value.
    pub fn raw(&self) -> u32 {
        (u32::from(self.major) << 16) | u32::from(self.minor)
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
    parse_reader(&mut binrw::io::Cursor::new(inflated))
}

/// Read an inflated chapter header from any seekable reader, e.g. one that logs its reads.
pub fn parse_reader<R: Read + Seek>(r: &mut R) -> Result<Header, super::Error> {
    let version = VersionBlock::read(r).map_err(header_error)?;
    let v = version.hundredths();
    if v < MIN_HUNDREDTHS {
        return Err(super::Error::Header {
            message: format!("save version {v} is older than 66.0"),
        });
    }
    let lobby = LobbySettings::read_args(r, (v,)).map_err(header_error)?;
    let mut ends = SectionEnds {
        lobby: position(r)?,
        ..SectionEnds::default()
    };
    // aoe2rec's AI layout fails on current single-player games with AI scripts, in ImHex too.
    let ai_start = position(r)?;
    let ai = match AiConfig::read(r) {
        Ok(ai) => ai,
        Err(e) => {
            r.seek(SeekFrom::Start(ai_start as u64))
                .map_err(|e| header_error(e.into()))?;
            return Err(match u32::read_le(r) {
                Ok(1) => super::Error::Unsupported {
                    what: "AI data (has_ai = 1)",
                    at: ai_start,
                },
                _ => header_error(e),
            });
        }
    };
    ends.ai = position(r)?;
    let replay = Replay::read(r).map_err(header_error)?;
    ends.replay = position(r)?;
    let map = MapInfo::read(r).map_err(header_error)?;
    ends.map = position(r)?;
    let restore_time = u32::read_le(r).map_err(header_error)?;
    let particles_count = u32::read_le(r).map_err(header_error)?;
    let particles = Vec::<u8>::read_le_args(
        r,
        binrw::VecArgs::builder()
            .count(particles_count as usize * 27)
            .finalize(),
    )
    .map_err(header_error)?;
    let identifier = u32::read_le(r).map_err(header_error)?;
    ends.players_start = position(r)?;
    let num_players = replay.num_players as usize;
    let ver = version.raw();
    let players = (0..num_players)
        .map(|_| {
            player::read_player(r, ver, num_players).map_err(|e| match e {
                player::ReadError::Bin(e) => header_error(e),
                player::ReadError::Unsupported { what, at } => {
                    super::Error::Unsupported { what, at }
                }
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
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

fn position<R: Seek>(r: &mut R) -> Result<usize, super::Error> {
    Ok(r.stream_position().map_err(|e| header_error(e.into()))? as usize)
}

fn header_error(e: binrw::Error) -> super::Error {
    super::Error::Header {
        message: e.to_string(),
    }
}
