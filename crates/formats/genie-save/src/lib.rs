//! The world save of Age of Empires II: Definitive Edition (save 66.x and later), read and written
//! to its last byte.
//!
//! The game saves its world in one format: the save-game file header ([`SaveGameFileHeader`]: the
//! version, the build, the lobby's `GameOptions`, the save's name and seed), then the world
//! (`World::save`, read back by `World::load_world`), the map and every player's objects included.
//! Three containers hold it:
//!
//! - a recorded game: every chapter starts with one, deflated (genie-rec reads the chapters);
//! - a saved game (`.aoe2spgame`, `.aoe2mpgame`);
//! - the tick oracle's snapshots (workspace FD-015), which the game writes through `World::save`.
//!
//! The layout is generated from build 158041's load routines (workspace `aoe2de-exe-re/layout`,
//! FD-014) into [`generated::world`]. Every field's boundary is checked against the game's own
//! reads, and a parsed save writes back byte for byte (genie-rec's `tests/de_oracle.rs` and
//! `tests/de_roundtrip.rs`, over the recordings' chapters). The generated version gates compare
//! the raw 16.16 save version ([`Header::raw_version`]).

// the 16.16 version's halves and the stream offsets (a save is far below 4 GiB) are cut on purpose
#![allow(clippy::cast_possible_truncation)]

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

/// Errors while reading a world save. Offsets are byte offsets into the save.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The bytes are not a save this reader takes: too old, cut short, or not the layout.
    #[error("world save: {message}")]
    Read {
        /// What went wrong, with where.
        message: String,
    },
    /// Writing a parsed save failed.
    #[error("world save written: {message}")]
    Write {
        /// What went wrong.
        message: String,
    },
    /// A part of the layout the generator does not derive yet.
    #[error("unsupported in this reader yet: {what} at save byte {at:#x}")]
    Unsupported {
        /// The part.
        what: &'static str,
        /// Where it starts in the save.
        at: usize,
    },
}

/// The version fields at the start of a save.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SaveVersion {
    /// The whole part, 66 for save 66.6.
    pub major: u16,
    /// A binary fraction: 39321 / 65536 is the ".6" of save 66.6.
    pub minor: u16,
    /// The engine build (`de.build`), the UI build + 0x20000.
    pub build: u32,
}

impl SaveVersion {
    /// The save version in hundredths, e.g. 6660 for 66.6.
    #[must_use]
    pub fn hundredths(&self) -> u32 {
        // the engine rounds the fraction to two decimals (66.6 -> 6660)
        u32::from(self.major) * 100 + (u32::from(self.minor) * 100 + 32768) / 65536
    }
}

/// Where the sections of the save end, for checks against other readers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SectionEnds {
    /// The save-game file header ends (aoe2rec's lobby settings); the world starts (aoe2rec's
    /// `ai_config` is the world's first field, `loadAIRules`, and the AI scripts it announces).
    pub lobby: usize,
    /// `BaseWorld`'s fields before the map (aoe2rec's `Replay`).
    pub replay: usize,
    /// The map ends.
    pub map: usize,
    /// The first player section starts here.
    pub players_start: usize,
}

/// A world save: the save-game file header, then the world (`World::load_world`), both generated
/// from the game's load code, to the save's last byte.
#[derive(Debug, Clone, PartialEq)]
pub struct Header {
    /// The save-game file header: version, build, the lobby's options.
    pub file_header: SaveGameFileHeader,
    /// The world, to the save's last byte.
    pub world: World,
    /// Where its sections end.
    pub ends: SectionEnds,
}

impl Header {
    /// The save version and build: 66.6 and 158041 for the corpus's tier 1.
    #[must_use]
    pub const fn save_version(&self) -> SaveVersion {
        let raw = self.raw_version();
        SaveVersion {
            major: (raw >> 16) as u16,
            minor: raw as u16,
            build: self.file_header.game_build_number,
        }
    }

    /// The version as the game compares it: 16.16 fixed point, e.g. `0x429999` for 66.6 (158041
    /// `DAT_143cd8948`, which the playback entry sets from this field). The generated readers'
    /// version gates take this value.
    #[must_use]
    pub const fn raw_version(&self) -> u32 {
        self.file_header.new_save_game_version
    }

    /// The player sections, Gaia first, with where each starts ([`PlayerSection::ends`] gives
    /// where it ends).
    #[must_use]
    pub fn players(&self) -> Vec<PlayerSection<'_>> {
        self.world
            .base_world
            .players
            .iter()
            .map(|p| PlayerSection {
                start: p.pos as usize,
                player: &p.val,
            })
            .collect()
    }
}

/// The bytes a value writes back: as many as it read.
pub(crate) fn written_len<T: binrw::BinWrite>(value: &T) -> usize
where
    for<'a> T::Args<'a>: Default,
{
    let mut out = binrw::io::Cursor::new(Vec::new());
    value
        .write_le(&mut out)
        .expect("a value read from a save writes back");
    out.into_inner().len()
}

/// Read a world save (a recording's chapter header once inflated).
///
/// # Errors
///
/// [`Error::Read`] for bytes that are not a save of 66.0 or later or do not end where the world
/// does, [`Error::Unsupported`] for a part the reader does not derive yet.
pub fn parse(save: &[u8]) -> Result<Header, Error> {
    parse_reader(&mut binrw::io::Cursor::new(save))
}

/// Read a world save from any seekable reader, e.g. one that logs its reads. The world reads to
/// the save's last byte; bytes left after it are an error.
///
/// # Errors
///
/// As [`parse`], and for a reader that fails.
pub fn parse_reader<R: Read + Seek>(r: &mut R) -> Result<Header, Error> {
    // The save version (16.16) at offset 12, peeked: the file header's own gates are decided for the
    // supported versions, and it brings the version the world's gates compare.
    let start = position(r)?;
    let mut peek = [0u8; 16];
    r.read_exact(&mut peek).map_err(|e| read_error(&e.into()))?;
    let raw = u32::from_le_bytes([peek[12], peek[13], peek[14], peek[15]]);
    let v = SaveVersion {
        major: (raw >> 16) as u16,
        minor: raw as u16,
        build: 0,
    }
    .hundredths();
    if v < MIN_HUNDREDTHS {
        return Err(Error::Read {
            message: format!("save version {v} is older than 66.0"),
        });
    }
    let _ = r
        .seek(SeekFrom::Start(start as u64))
        .map_err(|e| read_error(&e.into()))?;
    let file_header = SaveGameFileHeader::read_args(r, (raw,)).map_err(|e| read_error(&e))?;
    let lobby_end = position(r)?;
    let world =
        World::read_args(r, (file_header.new_save_game_version,)).map_err(
            |e| match unsupported_in(&e) {
                Some((what, at)) => Error::Unsupported { what, at },
                None => read_error(&e),
            },
        )?;
    let here = position(r)?;
    let end = r
        .seek(SeekFrom::End(0))
        .map_err(|e| read_error(&e.into()))? as usize;
    if here != end {
        return Err(Error::Read {
            message: format!(
                "the world ends at {here}, {} bytes before the save's end",
                end - here
            ),
        });
    }
    for p in &world.base_world.players {
        if p.val.world_player().is_none() {
            // the game's WorldPlayerFactory.cpp: "InvalidPlayerType: %d"
            return Err(Error::Read {
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

/// Write a save as `parse` read it, in read order. Read, then write gives back the save byte for
/// byte (genie-rec's `tests/de_roundtrip.rs`).
///
/// # Errors
///
/// [`Error::Write`] when the writer fails or a value cannot be written back.
pub fn write<W: binrw::io::Write + Seek>(h: &Header, w: &mut W) -> Result<(), Error> {
    use binrw::BinWrite;
    (|| {
        h.file_header.write_le(w)?;
        h.world.write_le(w)
    })()
    .map_err(|e| Error::Write {
        message: e.to_string(),
    })
}

fn position<R: Seek>(r: &mut R) -> Result<usize, Error> {
    Ok(r.stream_position().map_err(|e| read_error(&e.into()))? as usize)
}

fn read_error(e: &binrw::Error) -> Error {
    Error::Read {
        message: e.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_save_version_rounds_its_fraction_to_hundredths() {
        let v = SaveVersion {
            major: 66,
            minor: 39321,
            build: 158_041,
        };
        assert_eq!(v.hundredths(), 6660);
    }

    #[test]
    fn a_save_older_than_66_is_refused_before_its_header_is_read() {
        let mut save = b"VER 9.4\0".to_vec();
        save.extend((-1.0f32).to_le_bytes());
        save.extend(0x0041_0000u32.to_le_bytes());
        assert_eq!(
            parse(&save),
            Err(Error::Read {
                message: "save version 6500 is older than 66.0".into()
            })
        );
    }
}
