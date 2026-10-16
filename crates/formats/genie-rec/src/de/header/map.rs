//! The map (pattern: `MapInfo`, `MapTile`, `Tile`, `ObstructionData`).

use binrw::binrw;

#[binrw]
#[derive(Debug, Clone, PartialEq)]
#[brw(little)]
pub struct MapInfo {
    pub size_x: u32,
    pub size_y: u32,
    #[br(temp)]
    #[bw(try_calc = u32::try_from(zones.len()))]
    zone_num: u32,
    #[br(count = zone_num, args { inner: ((size_x * size_y) as usize,) })]
    pub zones: Vec<MapZone>,
    pub all_visible: u8,
    pub fog_of_war: u8,
    #[br(count = size_x * size_y)]
    pub tiles: Vec<Tile>,
    #[br(temp)]
    #[bw(try_calc = u32::try_from(obstructions.len()))]
    data_count: u32,
    pub unknown_count: u32,
    #[br(count = data_count * 4)]
    pub unknown2: Vec<u8>,
    #[br(count = data_count)]
    pub obstructions: Vec<Obstructions>,
    pub size_x_2: u32,
    pub size_y_2: u32,
    #[br(count = size_x * size_y * 2)]
    pub unknown6: Vec<u32>,
}

/// One zone layer (pattern: `MapTile`).
#[binrw]
#[derive(Debug, Clone, PartialEq)]
#[brw(little)]
#[br(import(tiles: usize))]
pub struct MapZone {
    pub tile_num: u32,
    #[br(count = 2044)]
    pub unknown1: Vec<u8>,
    #[br(count = tiles)]
    pub tile_zones: Vec<u16>,
    #[br(temp)]
    #[bw(try_calc = u32::try_from(unknown2.len()))]
    num_floats: u32,
    #[br(count = num_floats)]
    pub unknown2: Vec<f32>,
    pub unknown3: u32,
}

/// One map tile; a terrain type of 255 adds three bytes.
#[binrw]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[brw(little)]
pub struct Tile {
    pub terrain_type: u8,
    pub unknown1: i8,
    pub original_terrain_type: u8,
    pub elevation: u8,
    pub unknown2: i16,
    pub unknown3: i16,
    pub unknown4: i16,
    #[br(if(terrain_type == 255))]
    pub extended: Option<[u8; 3]>,
}

#[binrw]
#[derive(Debug, Clone, PartialEq, Eq)]
#[brw(little)]
pub struct Obstructions {
    #[br(temp)]
    #[bw(try_calc = u32::try_from(points.len() / 2))]
    count: u32,
    #[br(count = count * 2)]
    pub points: Vec<i32>,
}
