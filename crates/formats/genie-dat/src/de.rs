//! The data file of Age of Empires II: Definitive Edition, file versions VER 7.1 to VER 8.9.
//!
//! Ported from genieutils (github.com/Tapsa/genieutils, LGPL-3.0; the library the game ships with
//! Advanced Genie Editor in `Tools_Builds/docs/Source.7z`): every struct follows its class's
//! `serializeObject`, and every version gate compares genieutils' `GameVersion` ordinals, of
//! which the DE files take `GV_C14` (VER 7.1) to `GV_C32` (VER 8.9). Gates that are constant over that
//! range (the `AoE`, `AoK` and SWGB ones) are folded.
//!
//! Counts are kept as the file stores them, so a file reads and writes back byte for byte; a
//! caller that changes a list's length updates its count. The file on disk is raw-deflate
//! compressed: [`DatFile::read_from`] inflates, [`DatFile::to_bytes`] gives the inflated bytes.

use binrw::{BinRead, BinResult, BinWrite, Endian, binrw};
use flate2::read::DeflateDecoder;
// `Seek` stays unimported: binrw's `count` expansion calls `binrw::io::Seek::stream_position`,
// which an imported `Seek` would make an unnecessary qualification.
use std::io::{Cursor, Read, Write};

/// genieutils' `GameVersion` ordinal of a DE file: VER 7.1 is C14, VER 8.9 is C32.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(pub u8);

impl Version {
    /// The version of a file version string (`VER 7.1` to `VER 8.9`).
    #[must_use]
    pub fn of(file_version: &[u8; 8]) -> Option<Self> {
        const VERSIONS: [&[u8; 7]; 19] = [
            b"VER 7.1", b"VER 7.2", b"VER 7.3", b"VER 7.4", b"VER 7.5", b"VER 7.6", b"VER 7.7",
            b"VER 7.8", b"VER 7.9", b"VER 8.0", b"VER 8.1", b"VER 8.2", b"VER 8.3", b"VER 8.4",
            b"VER 8.5", b"VER 8.6", b"VER 8.7", b"VER 8.8", b"VER 8.9",
        ];
        VERSIONS
            .iter()
            .zip(14u8..)
            .find(|(v, _)| file_version[..7] == v[..])
            .map(|(_, n)| Self(n))
    }

    /// Whether the file is at least genieutils' `GV_C``n`.
    #[must_use]
    pub const fn at_least(self, n: u8) -> bool {
        self.0 >= n
    }
}

/// Why a file could not be read.
#[derive(Debug)]
pub enum Error {
    /// Reading the input failed, or it did not inflate.
    Io(std::io::Error),
    /// The file version is not one of VER 7.1 to VER 8.9.
    UnsupportedVersion([u8; 8]),
    /// The data does not follow the format, or writing it failed.
    Parse(binrw::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::UnsupportedVersion(v) => {
                write!(
                    f,
                    "unsupported file version {:?}",
                    String::from_utf8_lossy(v)
                )
            }
            Self::Parse(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<binrw::Error> for Error {
    fn from(e: binrw::Error) -> Self {
        Self::Parse(e)
    }
}

/// A string as DE stores it (`serializeDebugString`): the marker 0x0A60, a u16 length, the bytes.
#[binrw]
#[brw(little, magic = 0x0A60u16)]
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct DebugString {
    /// Number of bytes in `bytes`.
    pub len: u16,
    /// The string's bytes, without a terminating NUL.
    #[br(count = len)]
    pub bytes: Vec<u8>,
}

impl DebugString {
    /// The bytes as text; invalid UTF-8 sequences become U+FFFD.
    #[must_use]
    pub fn as_str(&self) -> std::borrow::Cow<'_, str> {
        String::from_utf8_lossy(&self.bytes)
    }
}

/// Read `pointers.len()` entries, one for each non-zero pointer (`serializeSubWithPointers`).
fn read_present<T, A, R>(
    reader: &mut R,
    endian: Endian,
    pointers: &[i32],
    args: A,
) -> BinResult<Vec<Option<T>>>
where
    T: for<'a> BinRead<Args<'a> = A>,
    A: Clone,
    R: Read + std::io::Seek,
{
    pointers
        .iter()
        .map(|&p| {
            (p != 0)
                .then(|| T::read_options(reader, endian, args.clone()))
                .transpose()
        })
        .collect()
}

/// Write the present entries, in order.
fn write_present<T, W>(list: &[Option<T>], writer: &mut W, endian: Endian) -> BinResult<()>
where
    T: for<'a> BinWrite<Args<'a> = ()>,
    W: Write + std::io::Seek,
{
    for entry in list.iter().flatten() {
        entry.write_options(writer, endian, ())?;
    }
    Ok(())
}

/// The whole file, inflated.
#[binrw]
#[brw(little)]
#[br(import(v: Version))]
#[derive(Clone, Debug, PartialEq)]
pub struct DatFile {
    /// The version string: `VER 7.1` to `VER 8.9`, then a NUL byte.
    pub file_version: [u8; 8],
    /// Number of terrain restrictions.
    pub terrain_restriction_count: i16,
    /// Number of terrains each terrain restriction has an entry for.
    pub terrains_used: i16,
    /// One in-memory pointer per terrain restriction, saved with the file.
    #[br(count = terrain_restriction_count)]
    pub float_ptr_terrain_tables: Vec<i32>,
    /// One in-memory pointer per terrain restriction, saved with the file.
    #[br(count = terrain_restriction_count)]
    pub terrain_pass_graphic_pointers: Vec<i32>,
    /// The terrain restrictions, indexed by restriction id.
    #[br(count = terrain_restriction_count, args { inner: (terrains_used,) })]
    pub terrain_restrictions: Vec<TerrainRestriction>,
    /// Number of player colours.
    pub player_colour_count: i16,
    /// The player colours.
    #[br(count = player_colour_count)]
    pub player_colours: Vec<PlayerColour>,
    /// Number of sounds.
    pub sound_count: i16,
    /// The sounds, indexed by sound id.
    #[br(count = sound_count)]
    pub sounds: Vec<Sound>,
    /// Number of graphic slots.
    pub graphic_count: i16,
    /// One pointer per graphic slot; a zero pointer marks an empty slot, which has no entry in
    /// `graphics`.
    #[br(count = graphic_count)]
    pub graphic_pointers: Vec<i32>,
    /// The graphics, indexed by graphic id; `None` where the slot's pointer is zero.
    #[br(parse_with = |r, e, (): ()| read_present(r, e, &graphic_pointers, ()))]
    #[bw(write_with = |l, w, e, (): ()| write_present(l, w, e))]
    pub graphics: Vec<Option<Graphic>>,
    /// The map settings and the terrain definitions.
    #[br(args(v))]
    pub terrain_block: TerrainBlock,
    /// The random map definitions.
    pub random_maps: RandomMaps,
    /// Number of tech effects.
    pub effect_count: i32,
    /// The tech effects, indexed by effect id.
    #[br(count = effect_count)]
    pub effects: Vec<Effect>,
    /// Number of unit headers (one per unit id).
    pub unit_header_count: i32,
    /// The shared task lists, indexed by unit id.
    #[br(count = unit_header_count, args { inner: (v,) })]
    pub unit_headers: Vec<UnitHeader>,
    /// Number of civilizations, Gaia included.
    pub civ_count: i16,
    /// The civilizations, indexed by civilization id (Gaia is 0).
    #[br(count = civ_count, args { inner: (v,) })]
    pub civs: Vec<Civ>,
    /// Number of techs.
    pub tech_count: i16,
    /// The techs, indexed by tech id.
    #[br(count = tech_count, args { inner: (v,) })]
    pub techs: Vec<Tech>,
    /// Not known yet; the first of seven engine-wide integers between the techs and the tech tree.
    pub time_slice: i32,
    /// Not known yet (an engine-wide integer, presumably for score keeping).
    pub unit_kill_rate: i32,
    /// Not known yet (an engine-wide integer, presumably for score keeping).
    pub unit_kill_total: i32,
    /// Not known yet (an engine-wide integer, presumably for score keeping).
    pub unit_hit_point_rate: i32,
    /// Not known yet (an engine-wide integer, presumably for score keeping).
    pub unit_hit_point_total: i32,
    /// Not known yet (an engine-wide integer, presumably for score keeping).
    pub razing_kill_rate: i32,
    /// Not known yet (an engine-wide integer, presumably for score keeping).
    pub razing_kill_total: i32,
    /// The tech tree screen's layout.
    #[br(args(v))]
    pub tech_tree: TechTree,
}

impl DatFile {
    /// Read a compressed file as the game ships it.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] if reading or inflating fails, and otherwise the errors of
    /// [`Self::from_inflated`].
    pub fn read_from(input: impl Read) -> Result<Self, Error> {
        let mut data = Vec::new();
        // The byte count is `data.len()`.
        let _ = DeflateDecoder::new(input).read_to_end(&mut data)?;
        Self::from_inflated(&data)
    }

    /// Read inflated bytes.
    ///
    /// # Errors
    ///
    /// Returns [`Error::UnsupportedVersion`] if the data does not start with `VER 7.1` to
    /// `VER 8.9`, and [`Error::Parse`] if the data does not follow the format.
    pub fn from_inflated(data: &[u8]) -> Result<Self, Error> {
        let mut version = [0u8; 8];
        version.copy_from_slice(data.get(..8).ok_or(Error::UnsupportedVersion([0; 8]))?);
        let v = Version::of(&version).ok_or(Error::UnsupportedVersion(version))?;
        Ok(Self::read_le_args(&mut Cursor::new(data), (v,))?)
    }

    /// The inflated bytes the file writes.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] if a field cannot be written.
    pub fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        let mut out = Cursor::new(Vec::new());
        self.write_le(&mut out)?;
        Ok(out.into_inner())
    }

    /// The file's version.
    ///
    /// # Panics
    ///
    /// Panics if `file_version` was changed to a string other than `VER 7.1` to `VER 8.9`; a file
    /// this module read always has a valid one.
    #[must_use]
    pub fn version(&self) -> Version {
        Version::of(&self.file_version).expect("checked when read")
    }
}

/// A terrain restriction: for each terrain, whether a unit using it may move or build there, and
/// the damage multiplier it takes there.
#[binrw]
#[brw(little)]
#[br(import(terrains: i16))]
#[derive(Clone, Debug, PartialEq)]
pub struct TerrainRestriction {
    /// One value per terrain: 0 is impassable, above 0 passable, above 0.05 also buildable; a unit
    /// whose armour uses this restriction takes damage multiplied by the value.
    #[br(count = terrains)]
    pub passable_buildable_dmg_multiplier: Vec<f32>,
    /// One set of walking graphics per terrain.
    #[br(count = terrains)]
    pub terrain_pass_graphics: Vec<TerrainPassGraphic>,
}

/// The graphics of a unit entering, leaving and walking on one terrain.
#[binrw]
#[brw(little)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerrainPassGraphic {
    /// Graphic shown when leaving the terrain.
    pub exit_tile_sprite_id: i32,
    /// Graphic shown when entering the terrain.
    pub enter_tile_sprite_id: i32,
    /// Graphic shown while walking on the terrain.
    pub walk_tile_sprite_id: i32,
    /// Rate of the walking graphic; its unit is not known yet.
    pub walk_sprite_rate: i32,
}

/// A player colour: the palette indices of a player's units, outlines, selection markers and
/// minimap dots.
#[binrw]
#[brw(little)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlayerColour {
    /// The colour's id.
    pub id: i32,
    /// First palette index of the colour's range of player-colour shades.
    pub player_color_base: i32,
    /// Palette index of the outline drawn for the player's units behind other objects.
    pub unit_outline_color: i32,
    /// Palette index of the first selection-marker colour.
    pub unit_selection_color1: i32,
    /// Palette index of the second selection-marker colour.
    pub unit_selection_color2: i32,
    /// Palette index of the player's dots on the minimap.
    pub minimap_colour: i32,
    /// Second minimap palette index; its use is not known yet.
    pub minimap_color2: i32,
    /// Third minimap palette index; its use is not known yet.
    pub minimap_color3: i32,
    /// Palette index of the player's text on the statistics screens.
    pub statistics_text: i32,
}

/// A sound: a set of sample files the game picks one of by probability.
#[binrw]
#[brw(little)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sound {
    /// The sound's id.
    pub id: i16,
    /// Delay before the sound plays; its unit is not known yet.
    pub play_delay: i16,
    /// Number of sample files in `items`.
    pub file_count: i16,
    /// How long a loaded sample stays cached; its unit is not known yet.
    pub cache_time: i32,
    /// The total the files' `probability` values are drawn against.
    pub total_probability: i16,
    /// The sample files.
    #[br(count = file_count)]
    pub items: Vec<SoundItem>,
}

/// One sample file of a [`Sound`].
#[binrw]
#[brw(little)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SoundItem {
    /// File name of the sample.
    pub file_name: DebugString,
    /// Id of the sample in the game's resource archives.
    pub resource_id: i32,
    /// Chance of picking this file, out of the sound's `total_probability`.
    pub probability: i16,
    /// Civilization the file is for; -1 for every civilization.
    pub civilization: i16,
    /// Architecture set the file is for; -1 for every set.
    pub icon_set: i16,
}

/// A graphic (sprite): its file, frames, angles, sounds and the deltas drawn with it.
#[binrw]
#[brw(little)]
#[derive(Clone, Debug, PartialEq)]
pub struct Graphic {
    /// Internal name.
    pub name: DebugString,
    /// File name of the sprite.
    pub file_name: DebugString,
    /// Name of the particle effect played with the graphic.
    pub particle_effect_name: DebugString,
    /// Id of the graphic's SLP in the resource archives; -1 for none.
    pub slp: i32,
    /// A runtime flag saved with the file; not used when reading.
    pub is_loaded: u8,
    /// A colour flag older versions used; not used in DE as far as known.
    pub old_color_flag: u8,
    /// Draw layer: higher layers draw on top, graphics on one layer by map position.
    pub layer: u8,
    /// Player colour forced on the graphic; -1 uses the owner's colour.
    pub player_color: i16,
    /// Whether clicks on transparent pixels select the object; its exact modes are not known yet.
    pub transparent_selection: u8,
    /// Bounding box in pixels: left, top, right, bottom.
    pub coordinates: [i16; 4],
    /// Number of deltas in `deltas`.
    pub delta_count: i16,
    /// Sound played with the graphic; -1 for none.
    pub sound_id: i16,
    /// Wwise event id of the graphic's sound.
    pub wwise_sound_id: u32,
    /// Non-zero if `angle_sounds` holds one entry per angle.
    pub angle_sounds_used: u8,
    /// Number of frames per angle.
    pub frame_count: i16,
    /// Number of facing angles; some of them are drawn mirrored.
    pub angle_count: i16,
    /// Multiplier on the speed of a unit showing the graphic.
    pub speed_multiplier: f32,
    /// Time each frame shows, in seconds.
    pub frame_duration: f32,
    /// Time before the animation starts again, in seconds.
    pub replay_delay: f32,
    /// Bit flags for how the animation plays (e.g. looping); not all known yet.
    pub sequence_type: u8,
    /// The graphic's id.
    pub id: i16,
    /// How the missing angles are mirrored from the stored ones.
    pub mirroring_mode: u8,
    /// A flag for the sprite editor; not used by the game as far as known.
    pub editor_flag: u8,
    /// Further graphics drawn with this one.
    #[br(count = delta_count)]
    pub deltas: Vec<GraphicDelta>,
    /// Per-angle sounds, present when `angle_sounds_used` is non-zero.
    #[br(count = if angle_sounds_used != 0 {
        usize::try_from(angle_count).unwrap_or(0)
    } else {
        0
    })]
    pub angle_sounds: Vec<GraphicAngleSound>,
}

/// A graphic drawn together with another one, at an offset.
#[binrw]
#[brw(little)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GraphicDelta {
    /// The graphic drawn; -1 for none.
    pub graphic_id: i16,
    /// Padding; kept to write the file back.
    pub padding1: i16,
    /// An in-memory pointer the game saved with the file; meaningless on disk, kept to write the
    /// file back.
    pub sprite_ptr: i32,
    /// Horizontal offset in pixels.
    pub offset_x: i16,
    /// Vertical offset in pixels.
    pub offset_y: i16,
    /// Angle at which the delta shows; -1 for every angle.
    pub display_angle: i16,
    /// Padding; kept to write the file back.
    pub padding2: i16,
}

/// The sounds of one angle of a graphic: up to three, each played at a frame.
#[binrw]
#[brw(little)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GraphicAngleSound {
    /// Frame at which the first sound plays.
    pub frame_num: i16,
    /// First sound; -1 for none.
    pub sound_id: i16,
    /// Wwise event id of the first sound.
    pub wwise_sound_id: u32,
    /// Frame at which the second sound plays.
    pub frame_num2: i16,
    /// Second sound; -1 for none.
    pub sound_id2: i16,
    /// Wwise event id of the second sound.
    pub wwise_sound_id2: u32,
    /// Frame at which the third sound plays.
    pub frame_num3: i16,
    /// Third sound; -1 for none.
    pub sound_id3: i16,
    /// Wwise event id of the third sound.
    pub wwise_sound_id3: u32,
}

/// genieutils' `SharedTerrain::TILE_TYPE_COUNT`.
const TILE_TYPES: usize = 19;
/// genieutils' `Terrain::getTerrainCount` for `GV_C8` and later.
const TERRAINS: usize = 200;
/// genieutils' `Terrain::TERRAIN_UNITS_SIZE`.
const TERRAIN_UNITS: usize = 30;

/// The map settings and the terrain definitions. Many fields are the in-memory map state the file
/// was saved from.
#[binrw]
#[brw(little)]
#[br(import(v: Version))]
#[derive(Clone, Debug, PartialEq)]
pub struct TerrainBlock {
    /// An in-memory pointer the game saved with the file.
    pub virtual_function_ptr: u32,
    /// An in-memory pointer the game saved with the file.
    pub map_pointer: u32,
    /// Map width in tiles (saved map state).
    pub map_width: i32,
    /// Map height in tiles (saved map state).
    pub map_height: i32,
    /// World width (saved map state).
    pub world_width: i32,
    /// World height (saved map state).
    pub world_height: i32,
    /// The size of each of the 19 tile shapes: flat and the slopes.
    pub tile_sizes: [TileSize; TILE_TYPES],
    /// Padding after the tile sizes; kept to write the file back.
    pub padding_ts: i16,
    /// The 200 terrain slots, indexed by terrain id.
    #[br(count = TERRAINS)]
    pub terrains: Vec<Terrain>,
    /// Lowest x of the map (saved map state).
    pub map_min_x: f32,
    /// Lowest y of the map (saved map state).
    pub map_min_y: f32,
    /// Highest x of the map (saved map state).
    pub map_max_x: f32,
    /// Highest y of the map (saved map state).
    pub map_max_y: f32,
    /// Highest x of the map plus one (saved map state).
    pub map_max_x_plus1: f32,
    /// Highest y of the map plus one (saved map state).
    pub map_max_y_plus1: f32,
    /// Number of terrains in use.
    pub terrains_used2: i16,
    /// Number of terrain borders in use.
    pub borders_used: i16,
    /// Not known yet; read as an i16 after the borders count.
    pub max_terrain: i16,
    /// Width of a tile in pixels.
    pub tile_width: i16,
    /// Height of a tile in pixels.
    pub tile_height: i16,
    /// Half the tile height in pixels.
    pub tile_half_height: i16,
    /// Half the tile width in pixels.
    pub tile_half_width: i16,
    /// Height of one elevation level in pixels.
    pub elev_height: i16,
    /// Current row of the map view (saved map state).
    pub cur_row: i16,
    /// Current column of the map view (saved map state).
    pub cur_col: i16,
    /// First row of the drawn block (saved map state).
    pub block_beg_row: i16,
    /// Last row of the drawn block (saved map state).
    pub block_end_row: i16,
    /// First column of the drawn block (saved map state).
    pub block_beg_col: i16,
    /// Last column of the drawn block (saved map state).
    pub block_end_col: i16,
    /// An in-memory pointer the game saved with the file.
    pub search_map_ptr: u32,
    /// An in-memory pointer the game saved with the file.
    pub search_map_rows_ptr: u32,
    /// Whether any terrain animation frame changed (saved map state).
    pub any_frame_change: u8,
    /// Whether the whole map is revealed (saved map state).
    pub map_visible_flag: u8,
    /// Whether fog of war is on (saved map state).
    pub fog_flag: u8,
}

/// The size of one tile shape (flat, or one of the slopes).
#[binrw]
#[brw(little)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct TileSize {
    /// Width in pixels.
    pub width: i16,
    /// Height in pixels.
    pub height: i16,
    /// Vertical offset in pixels.
    pub delta_y: i16,
}

/// The frames of a terrain's graphic that draw one tile shape.
#[binrw]
#[brw(little)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct FrameData {
    /// Number of frames.
    pub frame_count: i16,
    /// Number of angles.
    pub angle_count: i16,
    /// Index of the first frame.
    pub shape_id: i16,
}

/// A terrain definition.
#[binrw]
#[brw(little)]
#[derive(Clone, Debug, PartialEq)]
pub struct Terrain {
    /// Whether the slot holds a terrain.
    pub enabled: u8,
    /// Not known yet; read as a u8 flag.
    pub random: u8,
    /// Whether the terrain is water.
    pub is_water: u8,
    /// Whether the scenario editor hides the terrain.
    pub hide_in_editor: u8,
    /// Language string id of the terrain's name.
    pub string_id: i32,
    /// Internal name.
    pub name: DebugString,
    /// Name of the terrain's texture file.
    pub name2: DebugString,
    /// Id of the terrain's SLP; -1 for none.
    pub slp: i32,
    /// An in-memory pointer the game saved with the file.
    pub shape_ptr: i32,
    /// Sound of the terrain; -1 for none.
    pub sound_id: i32,
    /// Wwise event id that starts the terrain's sound.
    pub wwise_sound_id: u32,
    /// Wwise event id that stops the terrain's sound.
    pub wwise_sound_stop_id: u32,
    /// Blend priority: where two terrains meet, the higher one blends over the other.
    pub blend_priority: i32,
    /// Blend type: which set of blending masks the terrain uses.
    pub blend_type: i32,
    /// Name of the terrain's overlay mask texture.
    pub overlay_mask_name: DebugString,
    /// Minimap palette indices: high, medium and low.
    pub colors: [u8; 3],
    /// Minimap palette indices of cliffs: left and right.
    pub cliff_colors: [u8; 2],
    /// Not known yet; read as a terrain id (u8).
    pub passable_terrain: u8,
    /// Not known yet; read as a terrain id (u8).
    pub impassable_terrain: u8,
    /// Whether the terrain is animated.
    pub is_animated: u8,
    /// Number of animation frames.
    pub animation_frames: i16,
    /// Number of frames to pause after the last one.
    pub pause_frames: i16,
    /// Time between animation frames.
    pub interval: f32,
    /// Pause between animation loops.
    pub pause_between_loops: f32,
    /// Current animation frame (saved map state).
    pub frame: i16,
    /// Current frame to draw (saved map state).
    pub draw_frame: i16,
    /// Time the animation frame last changed (saved map state).
    pub animate_last: f32,
    /// Whether the frame changed since the terrain was drawn (saved map state).
    pub frame_changed: u8,
    /// Whether the terrain was drawn (saved map state).
    pub drawn: u8,
    /// The frames of each of the 19 tile shapes.
    pub elevation_graphics: [FrameData; TILE_TYPES],
    /// Terrain whose graphic is drawn in this one's place; -1 for its own.
    pub terrain_to_draw: i16,
    /// Rows and columns of the terrain's texture.
    pub terrain_dimensions: [i16; 2],
    /// Per terrain unit: a density whose difference to `terrain_unit_density` is not known yet.
    pub terrain_unit_masked_density: [i16; TERRAIN_UNITS],
    /// Units placed with the terrain (e.g. a forest's trees); -1 for none.
    pub terrain_unit_id: [i16; TERRAIN_UNITS],
    /// Per terrain unit: how densely it is placed.
    pub terrain_unit_density: [i16; TERRAIN_UNITS],
    /// Per terrain unit: whether it is centred on its tile.
    pub terrain_unit_centering: [u8; TERRAIN_UNITS],
    /// Number of the terrain-unit entries in use.
    pub number_of_terrain_units_used: i16,
    /// Not known yet; read as an i16.
    pub phantom: i16,
}

/// The random maps: their heads, then each map again with its lists (genieutils' `io_all_`).
#[binrw]
#[brw(little)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RandomMaps {
    /// Number of random maps.
    pub map_count: u32,
    /// An in-memory pointer the game saved with the file.
    pub random_maps_ptr: i32,
    /// Each map's settings and list counts.
    #[br(count = map_count)]
    pub heads: Vec<MapInfoHead>,
    /// Each map again, now with its lists.
    #[br(count = map_count)]
    pub maps: Vec<MapInfo>,
}

/// A random map's settings and the counts of its lists, without the lists.
#[binrw]
#[brw(little)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MapInfoHead {
    /// The map's id.
    pub map_id: i32,
    /// Border widths of the four map edges.
    pub borders: [i32; 4],
    /// How much of the borders is used; its scale is not known yet.
    pub border_usage: i32,
    /// Shape of the water; its values are not known yet.
    pub water_shape: i32,
    /// Terrain the map starts out as.
    pub base_terrain: i32,
    /// Share of the map covered by the lands, in percent.
    pub land_coverage: i32,
    /// An id the game does not use, as far as known.
    pub unused_id: i32,
    /// Number of lands.
    pub land_count: u32,
    /// An in-memory pointer the game saved with the file.
    pub lands_ptr: i32,
    /// Number of scattered terrains.
    pub terrain_count: u32,
    /// An in-memory pointer the game saved with the file.
    pub terrains_ptr: i32,
    /// Number of object groups.
    pub unit_count: u32,
    /// An in-memory pointer the game saved with the file.
    pub units_ptr: i32,
    /// Number of elevation entries.
    pub elevation_count: u32,
    /// An in-memory pointer the game saved with the file.
    pub elevations_ptr: i32,
}

/// A random map's settings with its lists of lands, terrains, objects and elevations.
#[binrw]
#[brw(little)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MapInfo {
    /// Border widths of the four map edges.
    pub borders: [i32; 4],
    /// How much of the borders is used; its scale is not known yet.
    pub border_usage: i32,
    /// Shape of the water; its values are not known yet.
    pub water_shape: i32,
    /// Terrain the map starts out as.
    pub base_terrain: i32,
    /// Share of the map covered by the lands, in percent.
    pub land_coverage: i32,
    /// An id the game does not use, as far as known.
    pub unused_id: i32,
    /// Number of lands.
    pub land_count: u32,
    /// An in-memory pointer the game saved with the file.
    pub lands_ptr: i32,
    /// The lands.
    #[br(count = land_count)]
    pub lands: Vec<MapLand>,
    /// Number of scattered terrains.
    pub terrain_count: u32,
    /// An in-memory pointer the game saved with the file.
    pub terrains_ptr: i32,
    /// The scattered terrains.
    #[br(count = terrain_count)]
    pub terrains: Vec<MapTerrain>,
    /// Number of object groups.
    pub unit_count: u32,
    /// An in-memory pointer the game saved with the file.
    pub units_ptr: i32,
    /// The object groups.
    #[br(count = unit_count)]
    pub units: Vec<MapUnit>,
    /// Number of elevation entries.
    pub elevation_count: u32,
    /// An in-memory pointer the game saved with the file.
    pub elevations_ptr: i32,
    /// The elevation entries.
    #[br(count = elevation_count)]
    pub elevations: Vec<MapElevation>,
}

/// A land of a random map: a player's or a shared area grown from a base.
#[binrw]
#[brw(little)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MapLand {
    /// The land's id.
    pub land_id: i32,
    /// Terrain of the land.
    pub terrain: u32,
    /// Distance kept to other lands.
    pub land_spacing: i32,
    /// Size of the land's base, the square it grows from.
    pub base_size: i32,
    /// Zone the land belongs to; lands of one zone may touch.
    pub zone: u8,
    /// How the land is placed; its values are not known yet.
    pub placement_type: u8,
    /// Padding; kept to write the file back.
    pub padding1: i16,
    /// Fixed x position of the base.
    pub base_x: i32,
    /// Fixed y position of the base.
    pub base_y: i32,
    /// Share of the land area, in percent.
    pub land_proportion: u8,
    /// Whether every player gets one such land.
    pub by_player_flag: u8,
    /// Padding; kept to write the file back.
    pub padding2: i16,
    /// Radius of the starting area.
    pub start_area_radius: i32,
    /// How the terrain fades out at the land's edge, in percent.
    pub terrain_edge_fade: i32,
    /// How clumped the land grows.
    pub clumpiness: i32,
}

/// A terrain a random map scatters in clumps.
#[binrw]
#[brw(little)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MapTerrain {
    /// Share of the map, in percent.
    pub proportion: i32,
    /// Terrain placed.
    pub terrain: i32,
    /// Number of clumps.
    pub clump_count: i32,
    /// Distance kept to other terrains.
    pub edge_spacing: i32,
    /// Terrain the clumps are placed on.
    pub placement_terrain: i32,
    /// How clumped the terrain grows.
    pub clumpiness: i32,
}

/// Objects a random map places in groups.
#[binrw]
#[brw(little)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MapUnit {
    /// Unit placed.
    pub unit: i32,
    /// Terrain the objects are placed on.
    pub host_terrain: i32,
    /// How the objects of a group are placed; its values are not known yet.
    pub group_placing: u8,
    /// Whether the number of groups scales with the map size.
    pub scale_flag: u8,
    /// Padding; kept to write the file back.
    pub padding1: i16,
    /// Number of objects in a group.
    pub objects_per_group: i32,
    /// Random variation of `objects_per_group`.
    pub fluctuation: i32,
    /// Number of groups (per player, or in total).
    pub groups_per_player: i32,
    /// Area a group spreads over.
    pub group_area: i32,
    /// Owner of the objects: -1 gives every player their own, 0 is Gaia.
    pub player_id: i32,
    /// Whether every player gets a set of the objects.
    pub set_place_for_all_players: i32,
    /// Minimum distance to the players.
    pub min_distance_to_players: i32,
    /// Maximum distance to the players.
    pub max_distance_to_players: i32,
}

/// Hills a random map raises.
#[binrw]
#[brw(little)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MapElevation {
    /// Share of the map, in percent.
    pub proportion: i32,
    /// Terrain raised.
    pub terrain: i32,
    /// Number of clumps.
    pub clump_count: i32,
    /// Terrain the elevation is placed on.
    pub base_terrain: i32,
    /// Elevation level.
    pub base_elevation: i32,
    /// Spacing between the elevated tiles.
    pub tile_spacing: i32,
}

/// A tech effect (genieutils' `Effect`, the "techage").
#[binrw]
#[brw(little)]
#[derive(Clone, Debug, PartialEq)]
pub struct Effect {
    /// Internal name.
    pub name: DebugString,
    /// Number of commands.
    pub command_count: i16,
    /// The commands, applied in order.
    #[br(count = command_count)]
    pub commands: Vec<EffectCommand>,
}

/// One command of a tech effect: its type and its parameters.
#[binrw]
#[brw(little)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EffectCommand {
    /// Command type (set, add or multiply an attribute or resource, enable or upgrade a unit, …).
    pub kind: u8,
    /// First parameter; its meaning depends on `kind` (for an attribute command, the unit).
    pub a: i16,
    /// Second parameter; its meaning depends on `kind` (for an attribute command, the class).
    pub b: i16,
    /// Third parameter; its meaning depends on `kind` (for an attribute command, the attribute).
    pub c: i16,
    /// The float parameter; its meaning depends on `kind` (usually the amount).
    pub d: f32,
}

/// The shared task list of a unit id (DE keeps the tasks in each unit's own list as well).
#[binrw]
#[brw(little)]
#[br(import(v: Version))]
#[derive(Clone, Debug, PartialEq)]
pub struct UnitHeader {
    /// Non-zero if the unit id has a task list.
    pub exists: u8,
    /// Number of tasks; present when `exists` is non-zero.
    #[br(if(exists != 0))]
    pub task_count: Option<i16>,
    /// The tasks.
    #[br(count = task_count.unwrap_or(0), args { inner: (v,) })]
    pub tasks: Vec<Task>,
}

/// A task: an action a unit can perform, with its targets, rates, graphics and sounds.
#[binrw]
#[brw(little)]
#[br(import(v: Version))]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Task {
    /// Task type: 0 generic, 1 tribe (the kind Age of Kings and later use).
    pub task_type: i16,
    /// The task's id, unique within its unit's task list.
    pub id: i16,
    /// Non-zero for the unit's default task.
    pub is_default: u8,
    /// Action the task performs (gather, build, attack, convert, …).
    pub action_type: i16,
    /// Class of the targets; -1 for any.
    pub class_id: i16,
    /// Target unit; -1 for any.
    pub unit_id: i16,
    /// Target terrain; -1 for any.
    pub terrain_id: i16,
    /// Resource the unit picks up while working.
    pub resource_in: i16,
    /// Resource whose amount multiplies what the unit gathers.
    pub resource_multiplier: i16,
    /// Resource the unit delivers.
    pub resource_out: i16,
    /// A resource field the game does not use, as far as known.
    pub unused_resource: i16,
    /// First work value; its meaning depends on the action.
    pub work_value1: f32,
    /// Second work value; its meaning depends on the action.
    pub work_value2: f32,
    /// Range at which the task works.
    pub work_range: f32,
    /// Whether the unit looks for targets of this task by itself.
    pub auto_search_targets: u8,
    /// Time between two target searches.
    pub search_wait_time: f32,
    /// Whether the task can be given by targeting an object.
    pub enable_targeting: u8,
    /// Combat level flag; its values are not known yet.
    pub combat_level_flag: u8,
    /// Gather type; its values are not known yet.
    pub gather_type: i16,
    /// Second work flag; its values are not known yet.
    pub work_flag2: i16,
    /// Which owners' objects may be targeted (own, allied, enemy, …).
    pub target_diplomacy: u8,
    /// Whether the target must still hold resources.
    pub carry_check: u8,
    /// Whether builders pick this task for a construction site.
    pub pick_for_construction: u8,
    /// Graphic while moving to the target.
    pub moving_graphic_id: i16,
    /// Graphic while proceeding with the work (e.g. the gathering swing).
    pub proceeding_graphic_id: i16,
    /// Graphic while working.
    pub working_graphic_id: i16,
    /// Graphic while carrying resources.
    pub carrying_graphic_id: i16,
    /// Sound while gathering.
    pub resource_gathering_sound_id: i16,
    /// Sound when delivering resources.
    pub resource_deposit_sound_id: i16,
    /// Wwise event id of the gathering sound.
    pub wwise_resource_gathering_sound_id: u32,
    /// Wwise event id of the delivery sound.
    pub wwise_resource_deposit_sound_id: u32,
    /// Whether the task is enabled; from VER 8.5 (`GV_C28`).
    #[br(if(v.at_least(28)))]
    pub enabled: Option<i16>,
}

/// A civilization: its starting resources and its own copy of every unit.
#[binrw]
#[brw(little)]
#[br(import(v: Version))]
#[derive(Clone, Debug, PartialEq)]
pub struct Civ {
    /// Player type; its values are not known yet.
    pub player_type: u8,
    /// Internal name.
    pub name: DebugString,
    /// Number of entries in `resources`.
    pub resource_count: i16,
    /// Effect that sets up the civilization's tech tree (what it cannot build); -1 for none.
    pub tech_tree_id: i16,
    /// Effect of the civilization's team bonus; -1 for none.
    pub team_bonus_id: i16,
    /// Starting value of every resource (player attribute).
    #[br(count = resource_count)]
    pub resources: Vec<f32>,
    /// Architecture set of the civilization's icons.
    pub icon_set: u8,
    /// Number of unit slots.
    pub unit_count: i16,
    /// One pointer per unit slot; a zero pointer marks an empty slot, which has no entry in
    /// `units`.
    #[br(count = unit_count)]
    pub unit_pointers: Vec<i32>,
    /// The civilization's units, indexed by unit id; `None` where the slot's pointer is zero.
    #[br(parse_with = |r, e, (): ()| read_present(r, e, &unit_pointers, (v,)))]
    #[bw(write_with = |l, w, e, (): ()| write_present(l, w, e))]
    pub units: Vec<Option<Unit>>,
}

/// genieutils' unit types (`UT_*`).
pub mod unit_type {
    /// Flags and other objects that move: the first type with a `speed`.
    pub const FLAG: u8 = 20;
    /// Dead units and fish: the first type with a movement part (`dead_fish`).
    pub const DEAD_FISH: u8 = 30;
    /// Birds: the first type with an action part and tasks (`bird`).
    pub const BIRD: u8 = 40;
    /// Combatants: the first type with a combat part (`type50`).
    pub const COMBATANT: u8 = 50;
    /// Projectiles: the only type with a projectile part.
    pub const PROJECTILE: u8 = 60;
    /// Trainable units: the first type with a creation part (`creatable`).
    pub const CREATABLE: u8 = 70;
    /// Buildings: the only type with a building part.
    pub const BUILDING: u8 = 80;
    /// Age of Empires' trees: despite the high number, only the part every unit has.
    pub const AOE_TREES: u8 = 90;
}

/// A unit definition: the part every unit type has, then the parts its `unit_type` reaches.
#[binrw]
#[brw(little)]
#[br(import(v: Version))]
#[derive(Clone, Debug, PartialEq)]
pub struct Unit {
    /// Unit type (see [`unit_type`]); it decides which of the optional parts follow.
    pub unit_type: u8,
    /// The unit's id.
    pub id: i16,
    /// Language string id of the name; from VER 7.2 (`GV_C15`).
    #[br(if(v.at_least(15)))]
    pub language_dll_name: Option<i32>,
    /// Language string id of the creation text; from VER 7.2 (`GV_C15`).
    #[br(if(v.at_least(15)))]
    pub language_dll_creation: Option<i32>,
    /// Language string id of the name as an i16; before VER 7.2 (`GV_C15`).
    #[br(if(!v.at_least(15)))]
    pub language_dll_name_u16: Option<i16>,
    /// Language string id of the creation text as an i16; before VER 7.2 (`GV_C15`).
    #[br(if(!v.at_least(15)))]
    pub language_dll_creation_u16: Option<i16>,
    /// Unit class (archer, infantry, building, tree, …).
    pub class: i16,
    /// The two standing graphics; -1 for none.
    pub standing_graphic: [i16; 2],
    /// Graphic while dying.
    pub dying_graphic: i16,
    /// Graphic of the undead state.
    pub undead_graphic: i16,
    /// Whether the unit has an undead state after dying.
    pub undead_mode: u8,
    /// Hit points; -1 makes the unit die at once.
    pub hit_points: i16,
    /// Line of sight, in tiles.
    pub line_of_sight: f32,
    /// Number of units that can garrison inside.
    pub garrison_capacity: u8,
    /// Radius of the collision box along x, y and z, in tiles.
    pub collision_size: [f32; 3],
    /// Sound when the unit is created.
    pub train_sound: i16,
    /// Sound when the unit is damaged.
    pub damage_sound: i16,
    /// Unit it turns into when it dies; -1 for none.
    pub dead_unit_id: i16,
    /// Unit spawned as its blood; -1 for none.
    pub blood_unit_id: i16,
    /// Draw order among objects on the same spot.
    pub sort_number: u8,
    /// Whether buildings may be placed over the unit.
    pub can_be_built_on: u8,
    /// Frame of the unit's icon.
    pub icon_id: i16,
    /// Whether the scenario editor hides the unit.
    pub hide_in_editor: u8,
    /// A portrait id older versions used; not used in DE as far as known.
    pub old_portrait_pict: i16,
    /// Whether the unit is available from the start rather than enabled by a tech.
    pub enabled: u8,
    /// Whether the unit is disabled.
    pub disabled: u8,
    /// Terrains needed beside the foundation (e.g. a dock's water); -1 for none.
    pub placement_side_terrain: [i16; 2],
    /// Terrains needed under the foundation; -1 for none.
    pub placement_terrain: [i16; 2],
    /// Free space needed to place the unit, along x and y.
    pub clearance_size: [f32; 2],
    /// How the unit may be placed on slopes.
    pub hill_mode: u8,
    /// How the unit shows under fog of war.
    pub fog_visibility: u8,
    /// Terrain restriction id: where the unit may be placed and move.
    pub terrain_restriction: i16,
    /// Whether the unit flies.
    pub fly_mode: u8,
    /// Amount of resources the unit can carry.
    pub resource_capacity: i16,
    /// Rate at which the unit's resources decay (e.g. a carcass rotting).
    pub resource_decay: f32,
    /// Blast defense level: which blast attacks damage the unit.
    pub blast_defense_level: u8,
    /// Combat level; its values are not known yet.
    pub combat_level: u8,
    /// What the player can do with the unit (select, move, attack, …).
    pub interaction_mode: u8,
    /// How the unit shows on the minimap.
    pub minimap_mode: u8,
    /// Which command buttons the unit shows.
    pub interface_kind: u8,
    /// Not known yet; read as an f32.
    pub multiple_attribute_mode: f32,
    /// Palette index of the unit's minimap dot.
    pub minimap_color: u8,
    /// Language string id of the help text.
    pub language_dll_help: i32,
    /// Language string id of the hotkey text.
    pub language_dll_hot_key_text: i32,
    /// Before VER 8.7 (`GV_C30`) the hotkey of the (single) train location sits here.
    #[br(if(!v.at_least(30)))]
    pub hot_key_id: Option<i32>,
    /// Whether the unit can be recycled; its exact effect is not known yet.
    pub recyclable: u8,
    /// Whether villagers gather from the unit by themselves.
    pub enable_auto_gather: u8,
    /// Whether the unit leaves a doppelganger in the fog when it dies.
    pub create_doppelganger_on_death: u8,
    /// Resource group gatherers treat the unit as.
    pub resource_gather_group: u8,
    /// Bit flags for how the unit is masked behind other objects.
    pub occlusion_mode: u8,
    /// Shape of the unit's obstruction (none, square, round, …).
    pub obstruction_type: u8,
    /// Obstruction class; its values are not known yet.
    pub obstruction_class: u8,
    /// Bit flags of the unit's traits; not all known yet.
    pub trait_: u8,
    /// Civilization the unit belongs to.
    pub civilization: u8,
    /// Not used as far as known; read as an i16.
    pub nothing: i16,
    /// Effect shown when the unit is selected.
    pub selection_effect: u8,
    /// Colour of the unit's selection in the scenario editor.
    pub editor_selection_colour: u8,
    /// Size of the selection outline along x, y and z.
    pub outline_size: [f32; 3],
    /// Two words genieutils writes as -2 and 0 and drops on read (`GV_CK` and later).
    pub unknown: [i32; 2],
    /// Three resources the unit holds or provides.
    pub resource_storages: [ResourceStorage; 3],
    /// Number of damage graphics.
    pub damage_graphic_count: u8,
    /// Graphics shown as the unit loses hit points.
    #[br(count = damage_graphic_count)]
    pub damage_graphics: Vec<DamageGraphic>,
    /// Sound when the unit is selected.
    pub selection_sound: i16,
    /// Sound when the unit dies.
    pub dying_sound: i16,
    /// Wwise event id of the creation sound.
    pub wwise_train_sound_id: u32,
    /// Wwise event id of the damage sound.
    pub wwise_damage_sound_id: u32,
    /// Wwise event id of the selection sound.
    pub wwise_selection_sound_id: u32,
    /// Wwise event id of the dying sound.
    pub wwise_dying_sound_id: u32,
    /// An attack reaction older versions used; not used in DE as far as known.
    pub old_attack_reaction: u8,
    /// A leftover flag for converting the terrain under the unit; not used as far as known.
    pub convert_terrain: u8,
    /// Internal name.
    pub name: DebugString,
    /// Unit this one was copied from.
    pub copy_id: i16,
    /// Base unit; its exact role is not known yet.
    pub base_id: i16,
    /// Movement speed; for unit types 20 and up, except 90.
    #[br(if(unit_type >= unit_type::FLAG && unit_type != unit_type::AOE_TREES))]
    pub speed: Option<f32>,
    /// Movement part; for unit types 30 and up, except 90.
    #[br(if(unit_type >= unit_type::DEAD_FISH && unit_type != unit_type::AOE_TREES))]
    pub dead_fish: Option<DeadFish>,
    /// Action part; for unit types 40 and up, except 90.
    #[br(if(unit_type >= unit_type::BIRD && unit_type != unit_type::AOE_TREES), args(v))]
    pub bird: Option<Bird>,
    /// Combat part; for unit types 50 and up, except 90.
    #[br(if(unit_type >= unit_type::COMBATANT && unit_type != unit_type::AOE_TREES), args(v))]
    pub type50: Option<Type50>,
    /// Projectile part; for unit type 60 only.
    #[br(if(unit_type == unit_type::PROJECTILE))]
    pub projectile: Option<Projectile>,
    /// Creation part; for unit types 70 and up, except 90.
    #[br(if(unit_type >= unit_type::CREATABLE && unit_type != unit_type::AOE_TREES), args(v))]
    pub creatable: Option<Creatable>,
    /// Building part; for unit type 80 only.
    #[br(if(unit_type == unit_type::BUILDING))]
    pub building: Option<Building>,
}

/// A resource a unit holds or provides (e.g. its food, or the population room it gives).
#[binrw]
#[brw(little)]
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct ResourceStorage {
    /// Resource id; -1 for none.
    pub kind: i16,
    /// Amount of the resource.
    pub amount: f32,
    /// How the resource is stored or given; its values are not known yet.
    pub flag: u8,
}

/// A graphic shown once a unit has lost a share of its hit points (e.g. fire on a building).
#[binrw]
#[brw(little)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DamageGraphic {
    /// The graphic.
    pub graphic_id: i16,
    /// Share of hit points lost, in percent, from which the graphic shows.
    pub damage_percent: i16,
    /// How the graphic is applied (over or instead of the unit's own); its values are not known
    /// yet.
    pub apply_mode: u8,
}

/// The movement part of a unit (genieutils' `DeadFish`, unit types 30 and up).
#[binrw]
#[brw(little)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DeadFish {
    /// Graphic while walking.
    pub walking_graphic: i16,
    /// Graphic while running.
    pub running_graphic: i16,
    /// Rotation speed.
    pub rotation_speed: f32,
    /// A size class older versions used; not used in DE as far as known.
    pub old_size_class: u8,
    /// Unit left behind as a trail (e.g. footprints); -1 for none.
    pub tracking_unit: i16,
    /// When the trail is left; its values are not known yet.
    pub tracking_unit_mode: u8,
    /// Spacing of the trail units.
    pub tracking_unit_density: f32,
    /// A move algorithm older versions used; not used in DE as far as known.
    pub old_move_algorithm: u8,
    /// Turn radius.
    pub turn_radius: f32,
    /// Speed while turning.
    pub turn_radius_speed: f32,
    /// Highest turn rate while moving.
    pub max_yaw_per_second_moving: f32,
    /// Time for a full turn while standing.
    pub stationary_yaw_revolution_time: f32,
    /// Highest turn rate while standing.
    pub max_yaw_per_second_stationary: f32,
    /// Smallest multiplier on the collision size; its use is not known yet.
    pub min_collision_size_multiplier: f32,
}

/// The action part of a unit (genieutils' `Bird`).
#[binrw]
#[brw(little)]
#[br(import(v: Version))]
#[derive(Clone, Debug, PartialEq)]
pub struct Bird {
    /// Id of the default task in `tasks`; -1 for none.
    pub default_task_id: i16,
    /// Radius in which the unit looks for targets.
    pub search_radius: f32,
    /// Work rate (e.g. gathering or building speed).
    pub work_rate: f32,
    /// From VER 7.8 (`GV_C21`) the drop sites are counted; before, there are three.
    #[br(if(v.at_least(21)))]
    pub drop_site_count: Option<i16>,
    /// Units the unit delivers resources to; -1 for none.
    #[br(count = drop_site_count.map_or(3, |n| usize::try_from(n).unwrap_or(0)))]
    pub drop_sites: Vec<i16>,
    /// Group of units that turn into each other with the task (e.g. villager kinds).
    pub task_swap_group: u8,
    /// Sound when a command starts.
    pub attack_sound: i16,
    /// Sound when a command ends (e.g. on arrival).
    pub move_sound: i16,
    /// Wwise event id of the command-start sound.
    pub wwise_attack_sound_id: u32,
    /// Wwise event id of the command-end sound.
    pub wwise_move_sound_id: u32,
    /// How the unit runs about (e.g. a fleeing animal).
    pub run_pattern: u8,
    /// Number of tasks; from VER 7.2 (`GV_C15`).
    #[br(if(v.at_least(15)))]
    pub task_count: Option<i16>,
    /// The unit's own tasks.
    #[br(count = task_count.unwrap_or(0), args { inner: (v,) })]
    pub tasks: Vec<Task>,
}

impl Bird {
    /// The unit's default task, from its own task list.
    #[must_use]
    pub fn default_task(&self) -> Option<&Task> {
        self.tasks.iter().find(|t| t.id == self.default_task_id)
    }
}

/// The combat part of a unit (genieutils' `Type50`).
#[binrw]
#[brw(little)]
#[br(import(v: Version))]
#[derive(Clone, Debug, PartialEq)]
pub struct Type50 {
    /// Armour against attack classes the unit has no armour entry for.
    pub base_armor: i16,
    /// Number of attacks.
    pub attack_count: i16,
    /// Attack amounts, one per armour class they hit.
    #[br(count = attack_count)]
    pub attacks: Vec<AttackOrArmor>,
    /// Number of armours.
    pub armour_count: i16,
    /// Armour amounts, one per class.
    #[br(count = armour_count)]
    pub armours: Vec<AttackOrArmor>,
    /// Terrain restriction whose values multiply the damage the unit takes on each terrain.
    pub defense_terrain_bonus: i16,
    /// Resistance to bonus damage; from VER 7.3 (`GV_C16`).
    #[br(if(v.at_least(16)))]
    pub bonus_damage_resistance: Option<f32>,
    /// Maximum attack range.
    pub max_range: f32,
    /// Radius of the blast damage.
    pub blast_width: f32,
    /// Time between attacks, in seconds.
    pub reload_time: f32,
    /// Projectile the unit fires; -1 for none.
    pub projectile_unit_id: i16,
    /// Chance to hit, in percent.
    pub accuracy_percent: i16,
    /// Not used as far as known; read as a u8.
    pub break_off_combat: u8,
    /// Frame of the attack graphic at which the attack lands or the projectile leaves.
    pub frame_delay: i16,
    /// Offset at which projectiles start, along x, y and z.
    pub graphic_displacement: [f32; 3],
    /// Blast attack level: which units the blast damages.
    pub blast_attack_level: u8,
    /// Minimum attack range.
    pub min_range: f32,
    /// Spread of the projectiles that miss.
    pub accuracy_dispersion: f32,
    /// Graphic while attacking.
    pub attack_graphic: i16,
    /// Melee armour the interface shows.
    pub displayed_melee_armour: i16,
    /// Attack the interface shows.
    pub displayed_attack: i16,
    /// Range the interface shows.
    pub displayed_range: f32,
    /// Reload time the interface shows.
    pub displayed_reload_time: f32,
    /// Share of the damage the blast deals; from VER 7.7 (`GV_C20`).
    #[br(if(v.at_least(20)))]
    pub blast_damage: Option<f32>,
    /// Share of the damage taken that is reflected to the attacker; from VER 7.9 (`GV_C22`).
    #[br(if(v.at_least(22)))]
    pub damage_reflection: Option<f32>,
    /// Share of the damage dealt to friendly units; from VER 7.9 (`GV_C22`).
    #[br(if(v.at_least(22)))]
    pub friendly_fire_damage: Option<f32>,
    /// Frame from which an attack can be interrupted; from VER 8.4 (`GV_C27`).
    #[br(if(v.at_least(27)))]
    pub interrupt_frame: Option<i16>,
    /// Firepower the unit adds when garrisoned; from VER 8.4 (`GV_C27`).
    #[br(if(v.at_least(27)))]
    pub garrison_firepower: Option<f32>,
    /// Second attack graphic; from VER 8.4 (`GV_C27`).
    #[br(if(v.at_least(27)))]
    pub attack_graphic2: Option<i16>,
}

/// An attack or armour amount for one armour class.
#[binrw]
#[brw(little)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AttackOrArmor {
    /// Armour class.
    pub class: i16,
    /// Amount.
    pub amount: i16,
}

/// The projectile part of a unit (unit type 60).
#[binrw]
#[brw(little)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Projectile {
    /// Projectile type (e.g. falling straight down); its values are not all known yet.
    pub projectile_type: u8,
    /// Whether the projectile aims at the target's predicted position.
    pub smart_mode: u8,
    /// Whether the projectile vanishes on hitting.
    pub hit_mode: u8,
    /// Whether the projectile's graphic passes through what it hits.
    pub vanish_mode: u8,
    /// Bit flags of special area effects; not known yet.
    pub area_effect_specials: u8,
    /// Arc of the flight.
    pub projectile_arc: f32,
}

/// One resource cost of a trainable unit.
#[binrw]
#[brw(little)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct ResourceCost {
    /// Resource id; -1 for none.
    pub kind: i16,
    /// Amount.
    pub amount: i16,
    /// Whether the amount is paid (non-zero) or only listed.
    pub flag: i16,
}

/// A unit or building that trains a unit, with the training time and button.
#[binrw]
#[brw(little)]
#[br(import(v: Version))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TrainLocation {
    /// Training time.
    pub queue_time: i16,
    /// Unit or building that trains the unit.
    pub location_id: i16,
    /// Position of the button on the command panel.
    pub button_id: u8,
    /// Hotkey id; from VER 8.7 (`GV_C30`).
    #[br(if(v.at_least(30)))]
    pub hot_key_id: Option<i32>,
}

/// The creation part of a unit (unit types 70 and up): costs, train locations, garrison
/// projectiles, charge and conversion settings.
#[binrw]
#[brw(little)]
#[br(import(v: Version))]
#[derive(Clone, Debug, PartialEq)]
pub struct Creatable {
    /// Up to three costs.
    pub resource_costs: [ResourceCost; 3],
    /// From VER 8.6 (`GV_C29`) the train locations are counted; before, there is one.
    #[br(if(v.at_least(29)))]
    pub train_location_count: Option<i16>,
    /// Where the unit is trained.
    #[br(
        count = train_location_count.map_or(1, |n| usize::try_from(n).unwrap_or(0)),
        args { inner: (v,) }
    )]
    pub train_locations: Vec<TrainLocation>,
    /// Damage multiplier for attacks from behind (the name of earlier games); DE's master loads
    /// it to `+0x8b4`, the save's `regeneration_rate`.
    pub rear_attack_modifier: f32,
    /// Damage multiplier for attacks from the side (the name of earlier games); DE's master loads
    /// it to `+0x8b8`, `mSpacingModifier`: a unit's radius in a group's formation is its
    /// `collision_size` on x times this (build 158041, `FUN_141025920`).
    pub flank_attack_modifier: f32,
    /// Kind of creatable unit; DE's `mFormationCategory` (`+0x8bc`): the sub-formation of a
    /// group the unit joins, 0 for units the order splitter never groups (villagers).
    pub creatable_type: u8,
    /// Hero bit flags (e.g. no conversion, healing); DE's `hero_flag` (`+0x8d9`).
    pub hero_mode: u8,
    /// Graphic shown while units are garrisoned.
    pub garrison_graphic: i32,
    /// Graphic when the unit is spawned.
    pub spawning_graphic: i16,
    /// Graphic when the unit is upgraded.
    pub upgrade_graphic: i16,
    /// Glow graphic of a hero; from VER 7.4 (`GV_C17`).
    #[br(if(v.at_least(17)))]
    pub hero_glow_graphic: Option<i16>,
    /// Idle graphic in combat; from VER 8.3 (`GV_C26`).
    #[br(if(v.at_least(26)))]
    pub idle_attack_graphic: Option<i16>,
    /// Highest charge of the unit's charge ability; from VER 7.3 (`GV_C16`).
    #[br(if(v.at_least(16)))]
    pub max_charge: Option<f32>,
    /// Charge regained over time; from VER 7.3 (`GV_C16`).
    #[br(if(v.at_least(16)))]
    pub recharge_rate: Option<f32>,
    /// Event that uses up the charge; from VER 7.3 (`GV_C16`).
    #[br(if(v.at_least(16)))]
    pub charge_event: Option<i16>,
    /// What the charge does (e.g. a charged attack); from VER 7.3 (`GV_C16`).
    #[br(if(v.at_least(16)))]
    pub charge_type: Option<i16>,
    /// What a charged ability targets; from VER 8.0 (`GV_C23`).
    #[br(if(v.at_least(23)))]
    pub charge_target: Option<i16>,
    /// Projectile of a charged attack; from VER 8.2 (`GV_C25`).
    #[br(if(v.at_least(25)))]
    pub charge_projectile_unit: Option<i32>,
    /// Priority as an attack target; from VER 8.2 (`GV_C25`).
    #[br(if(v.at_least(25)))]
    pub attack_priority: Option<u8>,
    /// Invulnerability level; from VER 8.2 (`GV_C25`).
    #[br(if(v.at_least(25)))]
    pub invulnerability_level: Option<f32>,
    /// Icon of the unit's ability button; from VER 8.1 (`GV_C24`).
    #[br(if(v.at_least(24)))]
    pub button_icon_id: Option<i16>,
    /// Language string id of that button's short tooltip; from VER 8.1 (`GV_C24`).
    #[br(if(v.at_least(24)))]
    pub button_short_tooltip_id: Option<i32>,
    /// Language string id of that button's long tooltip; from VER 8.1 (`GV_C24`).
    #[br(if(v.at_least(24)))]
    pub button_extended_tooltip_id: Option<i32>,
    /// Hotkey action of that button; from VER 8.1 (`GV_C24`).
    #[br(if(v.at_least(24)))]
    pub button_hotkey_action: Option<i16>,
    /// Modifier on the shortest time to convert the unit; from VER 7.6 (`GV_C19`).
    #[br(if(v.at_least(19)))]
    pub min_conversion_time_mod: Option<f32>,
    /// Modifier on the longest time to convert the unit; from VER 7.6 (`GV_C19`).
    #[br(if(v.at_least(19)))]
    pub max_conversion_time_mod: Option<f32>,
    /// Modifier on the chance to convert the unit; from VER 7.6 (`GV_C19`).
    #[br(if(v.at_least(19)))]
    pub conversion_chance_mod: Option<f32>,
    /// Projectiles fired with nothing garrisoned.
    pub total_projectiles: f32,
    /// Projectiles fired when fully garrisoned.
    pub max_total_projectiles: u8,
    /// Area the projectiles start from: width, length and randomness.
    pub projectile_spawning_area: [f32; 3],
    /// Projectile of the extra (garrison) projectiles; -1 for the main one.
    pub secondary_projectile_unit: i32,
    /// Graphic of the special ability.
    pub special_graphic: i32,
    /// Special ability: none, block, counter charge or charge.
    pub special_ability: u8,
    /// Pierce armour the interface shows.
    pub displayed_pierce_armour: i16,
}

/// A unit attached to a building, at an offset.
#[binrw]
#[brw(little)]
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct BuildingAnnex {
    /// The attached unit; -1 for none.
    pub unit_id: i16,
    /// Its offset from the building along x and y.
    pub misplacement: [f32; 2],
}

/// The building part of a unit (unit type 80).
#[binrw]
#[brw(little)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Building {
    /// Graphic while under construction.
    pub construction_graphic_id: i16,
    /// Graphic in snow.
    pub snow_graphic_id: i16,
    /// Graphic when destroyed.
    pub destruction_graphic_id: i16,
    /// Graphic of the rubble.
    pub destruction_rubble_graphic_id: i16,
    /// Graphic while researching.
    pub researching_graphic: i16,
    /// Graphic when a research completes.
    pub research_completed_graphic: i16,
    /// Whether the graphic adapts to adjacent buildings (walls).
    pub adjacent_mode: u8,
    /// Angle of the graphic.
    pub graphics_angle: i16,
    /// Whether the building disappears once built (e.g. a placer for a stack).
    pub disappears_when_built: u8,
    /// Building placed on top of this one; -1 for none.
    pub stack_unit_id: i16,
    /// Terrain laid under the building when built; -1 for none.
    pub foundation_terrain_id: i16,
    /// An overlay id from Age of Empires (roads); not used in DE as far as known.
    pub old_overlay_id: i16,
    /// Tech researched when the building completes (e.g. an age); -1 for none.
    pub tech_id: i16,
    /// Whether the building shows fire when damaged.
    pub can_burn: u8,
    /// Up to four attached units.
    pub annexes: [BuildingAnnex; 4],
    /// Building the unit is an annex of; -1 for none.
    pub head_unit: i16,
    /// Unit the building transforms into; -1 for none.
    pub transform_unit: i16,
    /// Sound when it transforms.
    pub transform_sound: i16,
    /// Sound while it is built.
    pub construction_sound: i16,
    /// Wwise event id of the transform sound.
    pub wwise_transform_sound_id: u32,
    /// Wwise event id of the construction sound.
    pub wwise_construction_sound_id: u32,
    /// Bit flags of the unit kinds that can garrison (villagers, infantry, …).
    pub garrison_type: u8,
    /// Rate at which garrisoned units heal.
    pub garrison_heal_rate: f32,
    /// Rate at which the building repairs itself with units garrisoned.
    pub garrison_repair_rate: f32,
    /// Not known yet; read as a unit id (i16).
    pub pile_unit: i16,
    /// Six looting entries; not known yet how the game uses them.
    pub looting_table: [u8; 6],
}

/// One resource cost of a tech.
#[binrw]
#[brw(little)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct ResearchResourceCost {
    /// Resource id; -1 for none.
    pub kind: i16,
    /// Amount.
    pub amount: i16,
    /// Whether the amount is paid (non-zero) or only listed.
    pub flag: u8,
}

/// A unit or building that researches a tech, with the research time and button (VER 8.8 and
/// later).
#[binrw]
#[brw(little)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResearchLocation {
    /// Unit or building that researches the tech.
    pub location_id: i16,
    /// Research time.
    pub queue_time: i16,
    /// Position of the button on the command panel.
    pub button_id: u8,
    /// Hotkey id.
    pub hot_key_id: i32,
}

/// A tech (genieutils' `Tech`, `Research.cpp`).
#[binrw]
#[brw(little)]
#[br(import(v: Version))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tech {
    /// Techs the tech requires; -1 for none.
    pub required_techs: [i16; 6],
    /// Up to three costs.
    pub resource_costs: [ResearchResourceCost; 3],
    /// How many of `required_techs` must be researched.
    pub required_tech_count: i16,
    /// Civilization the tech is for; -1 for every civilization.
    pub civ: i16,
    /// Whether the tech is available with the full tech tree setting.
    pub full_tech_mode: i16,
    /// Before VER 8.8 (`GV_C31`) one research location's fields are spread over the tech.
    #[br(if(!v.at_least(31)))]
    pub location_id: Option<i16>,
    /// Language string id of the name; from VER 7.5 (`GV_C18`).
    #[br(if(v.at_least(18)))]
    pub language_dll_name: Option<i32>,
    /// Language string id of the description; from VER 7.5 (`GV_C18`).
    #[br(if(v.at_least(18)))]
    pub language_dll_description: Option<i32>,
    /// Language string id of the name as an i16; before VER 7.5 (`GV_C18`).
    #[br(if(!v.at_least(18)))]
    pub language_dll_name_u16: Option<i16>,
    /// Language string id of the description as an i16; before VER 7.5 (`GV_C18`).
    #[br(if(!v.at_least(18)))]
    pub language_dll_description_u16: Option<i16>,
    /// Research time; before VER 8.8 (`GV_C31`).
    #[br(if(!v.at_least(31)))]
    pub queue_time: Option<i16>,
    /// Effect applied when the tech is researched; -1 for none.
    pub effect_id: i16,
    /// Tech type: 0 normal, 2 shown on the age progress bar.
    pub kind: i16,
    /// Frame of the tech's icon.
    pub icon_id: i16,
    /// Position of the button on the command panel; before VER 8.8 (`GV_C31`).
    #[br(if(!v.at_least(31)))]
    pub button_id: Option<u8>,
    /// Language string id of the help text.
    pub language_dll_help: i32,
    /// Language string id of the tech tree text.
    pub language_dll_tech_tree: i32,
    /// Hotkey id; before VER 8.8 (`GV_C31`).
    #[br(if(!v.at_least(31)))]
    pub hot_key_id: Option<i32>,
    /// Internal name.
    pub name: DebugString,
    /// Whether the tech can be researched more than once; from VER 7.2 (`GV_C15`).
    #[br(if(v.at_least(15)))]
    pub repeatable: Option<u8>,
    /// Number of research locations; from VER 8.8 (`GV_C31`).
    #[br(if(v.at_least(31)))]
    pub research_location_count: Option<i16>,
    /// Where the tech is researched.
    #[br(count = research_location_count.unwrap_or(0))]
    pub research_locations: Vec<ResearchLocation>,
}

/// genieutils' `TechTreeAge::getZoneCount` and `Common::getSlots` for `AoK` and later.
const ZONES: usize = 10;
/// genieutils' `BuildingConnection::AGES`.
const AGES: usize = 5;

/// The tech tree screen: its ages, buildings, units and researches.
#[binrw]
#[brw(little)]
#[br(import(v: Version))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TechTree {
    /// Number of ages.
    pub age_count: u8,
    /// Number of buildings.
    pub building_count: u8,
    /// From VER 8.9 (`GV_C32`) the unit connections are counted in an i16.
    #[br(if(v.at_least(32)))]
    pub unit_count_wide: Option<i16>,
    /// Number of units as a u8; before VER 8.9 (`GV_C32`).
    #[br(if(!v.at_least(32)))]
    pub unit_count: Option<u8>,
    /// Number of researches.
    pub research_count: u8,
    /// Number of unit and tech groups; its use is not known yet.
    pub total_unit_tech_groups: i32,
    /// The ages.
    #[br(count = age_count)]
    pub ages: Vec<TechTreeAge>,
    /// The buildings.
    #[br(count = building_count)]
    pub buildings: Vec<BuildingConnection>,
    /// The units.
    #[br(count = unit_count_wide.map_or_else(
        || usize::from(unit_count.unwrap_or(0)),
        |n| usize::try_from(n).unwrap_or(0),
    ))]
    pub units: Vec<UnitConnection>,
    /// The researches.
    #[br(count = research_count)]
    pub researches: Vec<ResearchConnection>,
}

/// The prerequisite slots every tech tree entry has.
#[binrw]
#[brw(little)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TechTreeCommon {
    /// Number of slots in use.
    pub slots_used: i32,
    /// Per slot: a unit or research id.
    pub unit_research: [i32; ZONES],
    /// Per slot: what the id is (e.g. unit, research, age).
    pub mode: [i32; ZONES],
}

/// Three counted id lists: buildings, units, techs.
#[binrw]
#[brw(little)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Connections {
    /// Number of buildings.
    pub building_count: u8,
    /// Building ids.
    #[br(count = building_count)]
    pub buildings: Vec<i32>,
    /// Number of units.
    pub unit_count: u8,
    /// Unit ids.
    #[br(count = unit_count)]
    pub units: Vec<i32>,
    /// Number of techs.
    pub tech_count: u8,
    /// Tech ids.
    #[br(count = tech_count)]
    pub techs: Vec<i32>,
}

/// An age on the tech tree.
#[binrw]
#[brw(little)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TechTreeAge {
    /// The age's id.
    pub id: i32,
    /// Status: 2 is the default, 3 not available; other values are not all known yet.
    pub status: u8,
    /// The buildings, units and techs of the age.
    pub connections: Connections,
    /// The prerequisite slots.
    pub common: TechTreeCommon,
    /// Number of building rows.
    pub num_building_levels: u8,
    /// Number of buildings in each zone.
    pub buildings_per_zone: [u8; ZONES],
    /// Group length in each zone.
    pub group_length_per_zone: [u8; ZONES],
    /// Not known yet; read as a u8.
    pub max_age_length: u8,
    /// Line mode; its values are not known yet.
    pub line_mode: i32,
}

/// A building on the tech tree.
#[binrw]
#[brw(little)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildingConnection {
    /// The building's unit id.
    pub id: i32,
    /// Status: 2 is the default, 3 not available; other values are not all known yet.
    pub status: u8,
    /// The buildings, units and techs the building makes available.
    pub connections: Connections,
    /// The prerequisite slots.
    pub common: TechTreeCommon,
    /// Age in which the building becomes available.
    pub location_in_age: u8,
    /// Number of units and techs in each age.
    pub units_techs_total: [u8; AGES],
    /// Number of units and techs in each age that come first in their line.
    pub units_techs_first: [u8; AGES],
    /// Line mode: 5 with connections, 6 without.
    pub line_mode: i32,
    /// Research that unlocks the building; -1 for none.
    pub enabling_research: i32,
}

/// A unit on the tech tree.
#[binrw]
#[brw(little)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnitConnection {
    /// The unit's id.
    pub id: i32,
    /// Status: 2 is the default, 3 not available; other values are not all known yet.
    pub status: u8,
    /// Building that trains the unit.
    pub upper_building: i32,
    /// The prerequisite slots.
    pub common: TechTreeCommon,
    /// Not known yet; read as an i32.
    pub vertical_line: i32,
    /// Number of units in `units`.
    pub unit_count: u8,
    /// Units the unit leads to.
    #[br(count = unit_count)]
    pub units: Vec<i32>,
    /// Position in its age: 0 hidden, 1 first, 2 second.
    pub location_in_age: i32,
    /// Research the unit requires; -1 for none.
    pub required_research: i32,
    /// Line mode: 2 first unit of its line, 3 depends on an earlier research.
    pub line_mode: i32,
    /// Research that unlocks the unit; -1 for none.
    pub enabling_research: i32,
}

/// A research on the tech tree.
#[binrw]
#[brw(little)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResearchConnection {
    /// The tech's id.
    pub id: i32,
    /// Status: 2 is the default, 3 not available; other values are not all known yet.
    pub status: u8,
    /// Building that researches the tech.
    pub upper_building: i32,
    /// The buildings, units and techs the research leads to.
    pub connections: Connections,
    /// The prerequisite slots.
    pub common: TechTreeCommon,
    /// Not known yet; read as an i32.
    pub vertical_line: i32,
    /// Position in its age.
    pub location_in_age: i32,
    /// Line mode; its values are not known yet.
    pub line_mode: i32,
}
