//! A player section (pattern: `Init`), read through its object lists.

use super::generated::player::WorldPlayer;
use super::main_object::MainObject;
use binrw::io::{Read, Seek};
use binrw::{BinRead, binread};

/// Where the parts of a player section end.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PlayerEnds {
    pub start: usize,
    pub enabled_objects: usize,
    pub initial_objects: usize,
    pub initial_map: usize,
    pub initial_state_restore: usize,
    pub objects_start: usize,
    /// The end of the last live object.
    pub objects_end: usize,
    /// The end of what this reader parsed of the section.
    pub end: usize,
}

/// One player's section.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerSection {
    pub ends: PlayerEnds,
    pub prelude: PlayerPrelude,
    pub tribe: super::tribe::TribeState,
    /// The tech tree (`tech_tree_help->save`, `techtree.cpp`); the pattern calls it the unlock tables.
    pub unlocks: Unlocks,
    pub tribe_end: super::tribe::TribeEnd,
    /// The `enabled` flags; their sum is the number of main objects.
    pub enabled_objects: Vec<u32>,
    pub main_objects: Vec<MainObject>,
    pub initial_map: InitialMap,
    pub state_restore: StateRestore,
    pub unknown23: u32,
    /// The live objects (the list the checkpoint sums over).
    pub objects: Vec<super::object::Object>,
    /// The lists after the live objects.
    pub tail: PlayerTail,
}

/// The sleeping and doppelganger object lists after a player's live objects.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerTail {
    pub sleeping_size: u32,
    pub sleeping_grow: u32,
    pub sleeping_objects: Vec<super::object::Object>,
    pub doppel_size: u32,
    pub doppel_grow: u32,
    pub doppel_objects: Vec<super::object::Object>,
    pub unknown24: [u32; 6],
}

/// The player fields before the tribe player's tech states: the player class byte, then the
/// generated part of the player load (`WorldPlayer.cpp`, which starts with `WorldPlayerBase.cpp`).
#[binread]
#[derive(Debug, Clone, PartialEq)]
#[br(little, import(ver: u32, num_players: usize))]
pub struct PlayerPrelude {
    /// `&type`, read by the player list's load (158041 `0x14167b900`) to pick the player class.
    pub player_type: u8,
    #[br(args(ver, num_players))]
    pub player: WorldPlayer,
}

/// The age, building, unit and tech unlock tables.
#[binread]
#[derive(Debug, Clone, PartialEq)]
#[br(little)]
pub struct Unlocks {
    #[br(temp)]
    available_ages: u8,
    #[br(temp)]
    available_buildings: u8,
    #[br(temp)]
    available_units: u8,
    #[br(temp)]
    available_techs: u8,
    #[br(count = available_ages, args { inner: (106,) })]
    pub ages: Vec<Unlock>,
    #[br(count = available_buildings, args { inner: (99,) })]
    pub buildings: Vec<Unlock>,
    pub unknown18: [u8; 4],
    #[br(count = available_units)]
    pub units: Vec<UnitUnlock>,
    #[br(count = available_techs)]
    pub techs: Vec<TechUnlock>,
}

/// An age or building unlock entry; the trailing unknown block is 106 bytes for ages, 99 for
/// buildings.
#[binread]
#[derive(Debug, Clone, PartialEq)]
#[br(little, import(tail: usize))]
pub struct Unlock {
    pub unknown1: i32,
    pub id: i32,
    pub unknown2: u8,
    #[br(temp)]
    n_buildings: u8,
    #[br(count = n_buildings)]
    pub buildings: Vec<i32>,
    #[br(temp)]
    n_units: u8,
    #[br(count = n_units)]
    pub units: Vec<i32>,
    #[br(temp)]
    n_techs: u8,
    #[br(count = n_techs)]
    pub techs: Vec<i32>,
    #[br(count = tail)]
    pub unknown3: Vec<u8>,
}

#[binread]
#[derive(Debug, Clone, PartialEq)]
#[br(little)]
pub struct UnitUnlock {
    pub unit_id: u32,
    pub unknown1: u8,
    pub production_building_id: i32,
    pub unknown3: u32,
    pub unknown4: u32,
    pub parent_unit_id: u32,
    pub unknown5: [u32; 19],
    #[br(temp)]
    linked_units_count: u8,
    #[br(count = linked_units_count)]
    pub linked_units: Vec<u32>,
    pub unknown6: i32,
    pub required_tech_id: i32,
    pub unknown8: i32,
    pub enabling_tech_id: i32,
}

#[binread]
#[derive(Debug, Clone, PartialEq)]
#[br(little)]
pub struct TechUnlock {
    pub tech_id: i32,
    pub unknown1: u8,
    pub building_id: i32,
    #[br(temp)]
    n_buildings: u8,
    #[br(count = n_buildings)]
    pub buildings: Vec<i32>,
    #[br(temp)]
    n_units: u8,
    #[br(count = n_units)]
    pub units: Vec<i32>,
    #[br(temp)]
    n_techs: u8,
    #[br(count = n_techs)]
    pub techs: Vec<i32>,
    #[br(count = 96)]
    pub unknown2: Vec<u8>,
}

#[binread]
#[derive(Debug, Clone, PartialEq)]
#[br(little)]
pub struct InitialMap {
    pub size_x: u32,
    pub size_y: u32,
    pub unknown1: u32,
    pub unknown2: u16,
    #[br(count = size_x * size_y)]
    pub tiles: Vec<i16>,
}

/// The restore table. The pattern reads restore objects until the next `0x0b` byte instead of
/// using `restore_sizes`; this port does the same.
#[binread]
#[derive(Debug, Clone, PartialEq)]
#[br(little)]
pub struct StateRestore {
    #[br(temp)]
    count: u32,
    #[br(count = count)]
    pub sizes: Vec<[u32; 2]>,
    #[br(parse_with = restore_objects)]
    pub objects: Vec<RestoreObject>,
}

#[binread]
#[derive(Debug, Clone, Copy, PartialEq)]
#[br(little)]
pub struct RestoreObject {
    pub unknown1: [u8; 4],
    pub unknown2: u32,
    pub unknown3: u32,
    pub unknown4: u32,
    pub unknown5: u8,
    pub unknown6: f32,
    pub unknown7: f32,
}

#[binrw::parser(reader, endian)]
fn restore_objects() -> binrw::BinResult<Vec<RestoreObject>> {
    let mut out = Vec::new();
    while super::primitives::peek_u8(reader)? != 0x0b {
        out.push(RestoreObject::read_options(reader, endian, ())?);
    }
    Ok(out)
}

/// Why a player section could not be read.
#[derive(Debug)]
pub enum ReadError {
    Bin(binrw::Error),
    /// A place where aoe2rec's pattern is known to fail (ImHex fails there too).
    Unsupported {
        what: &'static str,
        at: usize,
    },
}

impl From<binrw::Error> for ReadError {
    fn from(e: binrw::Error) -> Self {
        ReadError::Bin(e)
    }
}

fn objects_of<R: Read + Seek>(r: &mut R, v: u32) -> Result<Vec<super::object::Object>, ReadError> {
    super::object::read_objects(r, v).map_err(unsupported_or_bin)
}

fn position<R: Seek>(r: &mut R) -> Result<usize, ReadError> {
    Ok(r.stream_position().map_err(binrw::Error::Io)? as usize)
}

fn unsupported_or_bin(e: binrw::Error) -> ReadError {
    match super::object::unsupported_in(&e) {
        Some((what, at)) => ReadError::Unsupported { what, at },
        None => ReadError::Bin(e),
    }
}

fn read_tail<R: Read + Seek>(r: &mut R, v: u32) -> Result<PlayerTail, ReadError> {
    use super::primitives::expect;
    let sleeping_size = u32::read_le(r)?;
    let sleeping_grow = u32::read_le(r)?;
    let sleeping_objects = objects_of(r, v)?;
    expect(r, b"\x00\x0b")?;
    let doppel_size = u32::read_le(r)?;
    let doppel_grow = u32::read_le(r)?;
    let doppel_objects = objects_of(r, v)?;
    expect(r, b"\x00\x0b\x00\x0b")?;
    let unknown24 = <[u32; 6]>::read_le(r)?;
    expect(r, b"\x00\x0b")?;
    Ok(PlayerTail {
        sleeping_size,
        sleeping_grow,
        sleeping_objects,
        doppel_size,
        doppel_grow,
        doppel_objects,
        unknown24,
    })
}

/// Read one player section, from its prelude to the end of its tail lists. The next section starts
/// right after it. aoe2rec's pattern instead skips Gaia's tail and searches for the next player.
pub fn read_player<R: Read + Seek>(
    r: &mut R,
    v: u32,
    ver: u32,
    num_players: usize,
) -> Result<PlayerSection, ReadError> {
    use super::primitives::expect;
    let mut ends = PlayerEnds {
        start: position(r)?,
        ..PlayerEnds::default()
    };
    let prelude = PlayerPrelude::read_args(r, (ver, num_players))?;
    let tribe = super::tribe::TribeState::read_args(r, (v, num_players))?;
    let unlocks = Unlocks::read(r)?;
    let tribe_end = super::tribe::read_end(r, prelude.player_type).map_err(unsupported_or_bin)?;
    expect(r, super::tribe::MARKER)?;
    let count = u32::read_le(r)?;
    let enabled_objects = Vec::<u32>::read_le_args(
        r,
        binrw::VecArgs::builder().count(count as usize).finalize(),
    )?;
    ends.enabled_objects = position(r)?;
    let main_count: u32 = enabled_objects.iter().sum();
    expect(r, b"\x0b")?;
    let main_objects = (0..main_count)
        .map(|_| MainObject::read_args(r, (v,)))
        .collect::<Result<Vec<_>, _>>()
        .map_err(unsupported_or_bin)?;
    ends.initial_objects = position(r)?;
    expect(r, b"\x0b")?;
    let initial_map = InitialMap::read(r)?;
    ends.initial_map = position(r)?;
    let state_restore = StateRestore::read(r)?;
    ends.initial_state_restore = position(r)?;
    expect(r, b"\x0b\x00")?;
    let unknown23 = u32::read_le(r)?;
    expect(r, b"\x02\x00\x00")?;
    ends.objects_start = position(r)?;
    let objects = objects_of(r, v)?;
    ends.objects_end = position(r)?;
    expect(r, b"\x00\x0b")?;
    let tail = read_tail(r, v)?;
    ends.end = position(r)?;
    Ok(PlayerSection {
        ends,
        prelude,
        tribe,
        unlocks,
        tribe_end,
        enabled_objects,
        main_objects,
        initial_map,
        state_restore,
        unknown23,
        objects,
        tail,
    })
}
