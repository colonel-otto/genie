//! A player section (pattern: `Init`), read through its object lists.

use super::generated::player::{Object, PlayerObjects, WorldPlayer};
use binrw::io::{Read, Seek};
use binrw::{BinRead, binread};

/// Where the parts of a player section start and end.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PlayerEnds {
    pub start: usize,
    /// Where the player's objects start (WorldPlayer vtable slot 29: its masters, maps, objects).
    pub objects_start: usize,
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
    /// The player's masters, visible map and resources, its live, sleeping and doppelganger
    /// objects (generated from WorldPlayer vtable slot 29).
    pub objects: PlayerObjects,
}

impl PlayerSection {
    /// The live objects (the list the checkpoint sums over).
    pub fn live_objects(&self) -> impl Iterator<Item = &Object> {
        self.objects.objects.objects()
    }

    /// The sleeping objects.
    pub fn sleeping_objects(&self) -> impl Iterator<Item = &Object> {
        self.objects.sleeping_objects.objects()
    }
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

fn position<R: Seek>(r: &mut R) -> Result<usize, ReadError> {
    Ok(r.stream_position().map_err(binrw::Error::Io)? as usize)
}

/// Read one player section, from its prelude to the end of its tail lists. The next section starts
/// right after it. aoe2rec's pattern instead skips Gaia's tail and searches for the next player.
pub fn read_player<R: Read + Seek>(
    r: &mut R,
    v: u32,
    ver: u32,
    num_players: usize,
) -> Result<PlayerSection, ReadError> {
    let mut ends = PlayerEnds {
        start: position(r)?,
        ..PlayerEnds::default()
    };
    let prelude = PlayerPrelude::read_args(r, (ver, num_players))?;
    let tribe = super::tribe::TribeState::read_args(r, (v, num_players))?;
    let unlocks = Unlocks::read(r)?;
    let tribe_end = super::tribe::read_end(r, prelude.player_type).map_err(|e| {
        match super::tribe::unsupported_in(&e) {
            Some((what, at)) => ReadError::Unsupported { what, at },
            None => ReadError::Bin(e),
        }
    })?;
    ends.objects_start = position(r)?;
    let objects = PlayerObjects::read_args(r, (ver, num_players))?;
    ends.end = position(r)?;
    Ok(PlayerSection {
        ends,
        prelude,
        tribe,
        unlocks,
        tribe_end,
        objects,
    })
}
