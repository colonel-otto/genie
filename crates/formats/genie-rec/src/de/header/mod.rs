//! The world save at the start of a chapter, read and written to its last byte.
//!
//! Generated from build 158041's load routines (aoe2de-exe-re `layout`, workspace FD-014) into
//! [`generated::world`]: the save-game file header ([`SaveGameFileHeader`]: the version, the build,
//! the lobby's GameOptions, the save's name and seed), then the world (`World::load_world`), the map
//! included. Every field's boundary is checked against the game's own reads (tests/de_oracle.rs),
//! and a parsed header writes back byte for byte (tests/de_roundtrip.rs). The generated version
//! gates compare the raw 16.16 save version ([`Header::raw_version`]).

pub mod generated;
mod layout_types;
mod objects;
mod player;

pub use generated::world::{
    BaseWorld, GameOptions, Object, ObjectList, Player, PlayerInfo, PlayerObjects, RgeMap, RgeTile,
    SaveGameFileHeader, World, WorldPlayer,
};
pub use layout_types::{
    LenString16, LenString32, LenVec32, Positioned, STRING_SIGNATURE, SigString,
};
pub use layout_types::{Unsupported, UnsupportedFeature, unsupported_in};
pub use player::{PlayerEnds, PlayerSection};

use binrw::BinRead;
use binrw::io::{Read, Seek, SeekFrom};

/// The oldest save version this reader supports (66.0).
pub const MIN_HUNDREDTHS: u32 = 6600;

/// Where the sections of the header end, for checks against other readers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SectionEnds {
    /// The save-game file header ends (aoe2rec's lobby settings); the world starts (aoe2rec's
    /// `ai_config` is the world's first field, `loadAIRules`, and the AI scripts it announces).
    pub lobby: usize,
    /// BaseWorld's fields before the map (aoe2rec's `Replay`).
    pub replay: usize,
    pub map: usize,
    /// The first player section starts here.
    pub players_start: usize,
}

/// The header: the save-game file header, then the world (`World::load_world`), both generated
/// from the game's load code, to the header's last byte.
#[derive(Debug, Clone, PartialEq)]
pub struct Header {
    pub file_header: SaveGameFileHeader,
    pub world: World,
    pub ends: SectionEnds,
}

impl Header {
    /// The save version and build: 66.6 and 158041 for the corpus's tier 1.
    pub fn save_version(&self) -> super::SaveVersion {
        let raw = self.raw_version();
        super::SaveVersion {
            major: (raw >> 16) as u16,
            minor: raw as u16,
            build: self.file_header.game_build_number,
        }
    }

    /// The version as the game compares it: 16.16 fixed point, e.g. `0x429999` for 66.6 (158041
    /// `DAT_143cd8948`, which the playback entry sets from this field). The generated readers' version
    /// gates take this value.
    pub fn raw_version(&self) -> u32 {
        self.file_header.new_save_game_version
    }

    /// The player sections, Gaia first, with where each starts and ends.
    pub fn players(&self) -> Vec<PlayerSection<'_>> {
        self.world
            .base_world
            .players
            .iter()
            .map(|p| PlayerSection {
                ends: PlayerEnds {
                    start: p.pos as usize,
                    end: p.pos as usize + written_len(&p.val),
                },
                player: &p.val,
            })
            .collect()
    }
}

/// The bytes a value writes back: as many as it read.
fn written_len<T: binrw::BinWrite>(value: &T) -> usize
where
    for<'a> T::Args<'a>: Default,
{
    let mut out = binrw::io::Cursor::new(Vec::new());
    value
        .write_le(&mut out)
        .expect("a value read from a header writes back");
    out.into_inner().len()
}

/// Read an inflated chapter header.
pub fn parse(inflated: &[u8]) -> Result<Header, super::Error> {
    parse_reader(&mut binrw::io::Cursor::new(inflated))
}

/// Read an inflated chapter header from any seekable reader, e.g. one that logs its reads. The
/// world reads to the header's last byte; bytes left after it are an error.
pub fn parse_reader<R: Read + Seek>(r: &mut R) -> Result<Header, super::Error> {
    // The save version (16.16) at offset 12, peeked: the file header's own gates are decided for the
    // supported versions, and it brings the version the world's gates compare.
    let start = position(r)?;
    let mut peek = [0u8; 16];
    r.read_exact(&mut peek)
        .map_err(|e| header_error(e.into()))?;
    let raw = u32::from_le_bytes([peek[12], peek[13], peek[14], peek[15]]);
    let v = super::SaveVersion {
        major: (raw >> 16) as u16,
        minor: raw as u16,
        build: 0,
    }
    .hundredths();
    if v < MIN_HUNDREDTHS {
        return Err(super::Error::Header {
            message: format!("save version {v} is older than 66.0"),
        });
    }
    r.seek(SeekFrom::Start(start as u64))
        .map_err(|e| header_error(e.into()))?;
    let file_header = SaveGameFileHeader::read_args(r, (raw,)).map_err(header_error)?;
    let lobby_end = position(r)?;
    let world =
        World::read_args(r, (file_header.new_save_game_version,)).map_err(
            |e| match unsupported_in(&e) {
                Some((what, at)) => super::Error::Unsupported { what, at },
                None => header_error(e),
            },
        )?;
    let here = position(r)?;
    let end = r
        .seek(SeekFrom::End(0))
        .map_err(|e| header_error(e.into()))? as usize;
    if here != end {
        return Err(super::Error::Header {
            message: format!(
                "the world ends at {here}, {} bytes before the header's end",
                end - here
            ),
        });
    }
    for p in &world.base_world.players {
        if p.val.world_player().is_none() {
            // the game's WorldPlayerFactory.cpp: "InvalidPlayerType: %d"
            return Err(super::Error::Header {
                message: format!("InvalidPlayerType: {} at {}", p.val.type_, p.pos),
            });
        }
    }
    let map = &world.base_world.map;
    let ends = SectionEnds {
        lobby: lobby_end,
        replay: map.pos as usize,
        map: map.pos as usize + written_len(&map.val),
        players_start: world
            .base_world
            .players
            .first()
            .map_or(here, |p| p.pos as usize),
    };
    Ok(Header {
        file_header,
        world,
        ends,
    })
}

/// Write a header as `parse` read it, in read order. Read, then write gives back the inflated
/// header byte for byte (tests/de_roundtrip.rs).
pub fn write<W: binrw::io::Write + Seek>(h: &Header, w: &mut W) -> Result<(), super::Error> {
    use binrw::BinWrite;
    (|| {
        h.file_header.write_le(w)?;
        h.world.write_le(w)
    })()
    .map_err(header_error)
}

fn position<R: Seek>(r: &mut R) -> Result<usize, super::Error> {
    Ok(r.stream_position().map_err(|e| header_error(e.into()))? as usize)
}

fn header_error(e: binrw::Error) -> super::Error {
    super::Error::Header {
        message: e.to_string(),
    }
}
