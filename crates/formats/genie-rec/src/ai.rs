//! Read and write player AI state.
//!
//! A computer player's AI state is saved as a chain of modules (main, build, construction,
//! diplomacy, emotional, information, resource, strategy). The field names follow the genie-rs
//! reader and the AI's own terms; values whose meaning is not known say so.

use crate::element::{ReadableHeaderElement, WritableHeaderElement};
use crate::reader::RecordingHeaderReader;
use crate::unit::Waypoint;
use crate::{ObjectID, PlayerID, Result};
use byteorder::{LE, ReadBytesExt, WriteBytesExt};
use genie_support::{ReadSkipExt, ReadStringsExt, UnitTypeID, read_opt_u16, read_opt_u32};
use std::convert::TryInto;
use std::io::{Read, Write};

/// The main AI module.
#[derive(Debug, Default, Clone)]
pub struct MainAI {
    /// The object IDs the main module lists.
    pub objects: Vec<ObjectID>,
}

impl ReadableHeaderElement for MainAI {
    fn read_from<R: Read>(input: &mut RecordingHeaderReader<R>) -> Result<Self> {
        let num_objects = input.read_u32::<LE>()?;
        let mut objects = vec![];
        for _ in 0..num_objects {
            objects.push(input.read_u32::<LE>()?.into());
        }
        Ok(Self { objects })
    }
}

/// One entry of the build AI's build list: something the AI wants to build or train.
#[derive(Debug, Default, Clone)]
pub struct BuildItem {
    /// The item's name.
    pub name: Option<String>,
    /// The unit type to build.
    pub type_id: u32,
    /// The ID of the item's object in the game, as the reader names it; not verified.
    pub game_id: u32,
    /// The item's size along x, y and z.
    pub size: (f32, f32, f32),
    /// The item's skip counter.
    pub skip: u32,
    /// The item's build category.
    pub build_category: u32,
    /// Whether the item is being built, as the reader names it.
    pub in_progress: u32,
    /// Whether the item has been built, as the reader names it.
    pub built: u32,
    /// How often the AI tried to build the item.
    pub build_attempts: u32,
    /// Where the item is built from, as the reader names it; not verified.
    pub build_from: u32,
    /// The terrain set the item may be placed on.
    pub terrain_set: u32,
    /// The item's two terrain adjacency values.
    pub terrain_adjacency: (u32, u32),
    /// Whether the item is placed on elevation, as the reader names it.
    pub place_on_elevation: u32,
    /// Whether the item uses `UserPatch`'s forward placement. Stored after version 10.87, false
    /// before.
    pub is_forward: bool,
}

impl ReadableHeaderElement for BuildItem {
    fn read_from<R: Read>(input: &mut RecordingHeaderReader<R>) -> Result<Self> {
        let mut item = Self {
            name: input.read_u32_length_prefixed_str()?,
            type_id: input.read_u32::<LE>()?,
            ..Default::default()
        };

        let _a2 = input.read_u32::<LE>()?;
        item.game_id = input.read_u32::<LE>()?;
        let _v21 = input.read_u32::<LE>()?;
        let _v23 = input.read_u32::<LE>()?;
        let _v25 = input.read_u32::<LE>()?;
        item.size = (
            input.read_f32::<LE>()?,
            input.read_f32::<LE>()?,
            input.read_f32::<LE>()?,
        );
        item.skip = input.read_u32::<LE>()?;
        item.build_category = input.read_u32::<LE>()?;
        item.in_progress = input.read_u32::<LE>()?;
        item.built = input.read_u32::<LE>()?;
        item.build_attempts = input.read_u32::<LE>()?;
        item.build_from = input.read_u32::<LE>()?;
        item.terrain_set = input.read_u32::<LE>()?;
        item.terrain_adjacency = (input.read_u32::<LE>()?, input.read_u32::<LE>()?);
        item.place_on_elevation = input.read_u32::<LE>()?;
        let _v27 = input.read_u32::<LE>()?;
        let _v12 = input.read_u32::<LE>()?;
        let _v29 = input.read_u32::<LE>()?;
        let _v31 = input.read_u8()?;
        item.is_forward = if input.version() > 10.87 {
            input.read_u32::<LE>()? != 0
        } else {
            false
        };
        Ok(item)
    }
}

/// The build AI module: the build list and what the AI has queued.
#[derive(Debug, Default, Clone)]
pub struct BuildAI {
    /// The name of the build list.
    pub build_list_name: Option<String>,
    /// The name of the build item requested last.
    pub last_build_item_requested: Option<String>,
    /// The name of the build item requested now.
    pub current_build_item_requested: Option<String>,
    /// The name of the build item to request next.
    pub next_build_item_requested: Option<String>,
    /// The items of the build list.
    pub build_queue: Vec<BuildItem>,
    /// Queued counts per object type (600 entries).
    pub queued_object_count: Vec<u32>,
    /// The number of buildings queued.
    pub queued_building_count: u32,
    /// The number of units queued.
    pub queued_unit_count: u32,
}

impl ReadableHeaderElement for BuildAI {
    fn read_from<R: Read>(input: &mut RecordingHeaderReader<R>) -> Result<Self> {
        let mut ai = Self::default();
        let build_list_len = input.read_u32::<LE>()?;
        ai.build_list_name = input.read_u32_length_prefixed_str()?;
        ai.last_build_item_requested = input.read_u32_length_prefixed_str()?;
        ai.current_build_item_requested = input.read_u32_length_prefixed_str()?;
        ai.next_build_item_requested = input.read_u32_length_prefixed_str()?;
        let _items_into_build_queue = if input.version() > 11.02 {
            input.read_u32::<LE>()?
        } else {
            build_list_len
        };

        let num_build_items = input.read_u32::<LE>()?;
        for _ in 0..num_build_items {
            ai.build_queue.push(BuildItem::read_from(input)?);
        }

        for _ in 0..600 {
            ai.queued_object_count.push(input.read_u32::<LE>()?);
        }
        ai.queued_building_count = input.read_u32::<LE>()?;
        ai.queued_unit_count = input.read_u32::<LE>()?;
        Ok(ai)
    }
}

/// A construction lot of the construction AI: a place where the AI plans a building.
#[derive(Debug, Default, Clone)]
pub struct ConstructionItem {
    /// The lot's name.
    pub name: Option<String>,
    /// The unit type planned for the lot.
    pub type_id: u32,
    /// The lot's X position.
    pub x: f32,
    /// The lot's Y position.
    pub y: f32,
    /// The lot's Z position.
    pub z: f32,
    /// The lot's size along X.
    pub xs: f32,
    /// The lot's size along Y.
    pub ys: f32,
    /// The lot's size along Z.
    pub zs: f32,
    /// The lot's skip counter.
    pub skip: u32,
    /// Whether the building on the lot is in progress, as the reader names it.
    pub in_progress: u32,
    /// Whether the building on the lot is built, as the reader names it.
    pub built: u32,
}

impl ReadableHeaderElement for ConstructionItem {
    fn read_from<R: Read>(input: &mut RecordingHeaderReader<R>) -> Result<Self> {
        let mut item = Self {
            name: input.read_u32_length_prefixed_str()?,
            type_id: input.read_u32::<LE>()?,
            ..Default::default()
        };

        let _a2 = input.read_u32::<LE>()?;
        let _v27 = input.read_u32::<LE>()?;
        item.x = input.read_f32::<LE>()?;
        item.y = input.read_f32::<LE>()?;
        item.z = input.read_f32::<LE>()?;
        item.xs = input.read_f32::<LE>()?;
        item.ys = input.read_f32::<LE>()?;
        item.zs = input.read_f32::<LE>()?;
        item.skip = input.read_u32::<LE>()?;
        item.in_progress = input.read_u32::<LE>()?;
        item.built = input.read_u32::<LE>()?;
        let _v23 = input.read_u32::<LE>()?;
        Ok(item)
    }
}

/// The construction AI module: the construction plan and its lots.
#[derive(Debug, Default, Clone)]
pub struct ConstructionAI {
    /// The name of the construction plan.
    pub plan_name: Option<String>,
    /// The plan's reference point (x, y, z).
    pub reference_point: (f32, f32, f32),
    /// The map size in tiles (x, y).
    pub map_size: (u32, u32),
    /// The plan's construction lots.
    pub construction_lots: Vec<ConstructionItem>,
    /// The lots placed at random positions.
    pub random_construction_lots: Vec<ConstructionItem>,
}

impl ReadableHeaderElement for ConstructionAI {
    fn read_from<R: Read>(input: &mut RecordingHeaderReader<R>) -> Result<Self> {
        let mut ai = Self::default();
        let num_lots = input.read_u32::<LE>()?;
        ai.plan_name = input.read_u32_length_prefixed_str()?;
        ai.reference_point = (
            input.read_f32::<LE>()?,
            input.read_f32::<LE>()?,
            input.read_f32::<LE>()?,
        );
        ai.map_size = (input.read_u32::<LE>()?, input.read_u32::<LE>()?);
        for _ in 0..num_lots {
            ai.construction_lots
                .push(ConstructionItem::read_from(input)?);
        }
        let num_lots = input.read_u32::<LE>()?;
        for _ in 0..num_lots {
            ai.random_construction_lots
                .push(ConstructionItem::read_from(input)?);
        }
        Ok(ai)
    }
}

/// The diplomacy AI module: how the AI feels about each player.
#[derive(Debug, Default, Clone, Copy)]
pub struct DiplomacyAI {
    /// How much this AI dislikes each player, 0-100.
    pub dislike: [u32; 10],
    /// How much this AI likes each player, 0-100.
    pub like: [u32; 10],
    /// Whether the AI may change its stance towards each player.
    pub changeable: [u8; 10],
}

impl ReadableHeaderElement for DiplomacyAI {
    fn read_from<R: Read>(input: &mut RecordingHeaderReader<R>) -> Result<Self> {
        let mut ai = Self::default();
        for i in 0..10 {
            ai.dislike[i] = input.read_u32::<LE>()?;
            ai.like[i] = input.read_u32::<LE>()?;
            ai.changeable[i] = input.read_u8()?;
        }
        Ok(ai)
    }
}

/// The emotional AI module.
#[derive(Debug, Default, Clone, Copy)]
pub struct EmotionalAI {
    /// The AI's six emotional state values.
    pub state: [u32; 6],
}

impl ReadableHeaderElement for EmotionalAI {
    fn read_from<R: Read>(input: &mut RecordingHeaderReader<R>) -> Result<Self> {
        let mut ai = Self::default();
        input.read_u32_into::<LE>(&mut ai.state)?;
        Ok(ai)
    }
}

/// What the information AI remembers about an important object.
#[derive(Debug, Default, Clone, Copy)]
pub struct ImportantObjectMemory {
    /// The object's ID; `None` marks an empty slot.
    pub id: Option<ObjectID>,
    /// The object's unit type.
    pub unit_type_id: Option<UnitTypeID>,
    /// The object's unit class.
    pub unit_class: Option<u16>,
    /// The object's last known tile position (x, y, z).
    pub location: (u8, u8, u8),
    /// The object's owner.
    pub owner: PlayerID,
    /// The object's last known hit points.
    pub hit_points: u16,
    /// How often the AI attacked the object.
    pub attack_attempts: u32,
    /// The number of kills the AI attributes to the object.
    pub kills: u8,
    /// The object's damage capability, as the AI estimates it.
    pub damage_capability: f32,
    /// The object's rate of fire.
    pub rate_of_fire: f32,
    /// The object's attack range.
    pub range: f32,
    /// The game time when the AI last saw the object, if it did.
    pub time_seen: Option<u32>,
    /// Whether the object is garrisoned (non-zero).
    pub is_garrisoned: u32,
}

impl ReadableHeaderElement for ImportantObjectMemory {
    fn read_from<R: Read>(input: &mut RecordingHeaderReader<R>) -> Result<Self> {
        Ok(Self {
            id: read_opt_u32(input)?,
            unit_type_id: read_opt_u16(input)?,
            unit_class: read_opt_u16(input)?,
            location: (input.read_u8()?, input.read_u8()?, input.read_u8()?),
            owner: input.read_u8()?.into(),
            hit_points: input.read_u16::<LE>()?,
            attack_attempts: input.read_u32::<LE>()?,
            kills: input.read_u8()?,
            damage_capability: input.read_f32::<LE>()?,
            rate_of_fire: input.read_f32::<LE>()?,
            range: input.read_f32::<LE>()?,
            time_seen: read_opt_u32(input)?,
            is_garrisoned: input.read_u32::<LE>()?,
        })
    }
}

/// A building lot of the information AI: a place reserved for a building.
#[derive(Debug, Default, Clone, Copy)]
pub struct BuildingLot {
    /// The building type planned for the lot; `None` marks an empty slot.
    pub unit_type_id: Option<UnitTypeID>,
    /// The lot's status.
    pub status: u8,
    /// The lot's tile position (x, y).
    pub location: (u8, u8),
}

impl ReadableHeaderElement for BuildingLot {
    fn read_from<R: Read>(input: &mut RecordingHeaderReader<R>) -> Result<Self> {
        let unit_type_id = read_opt_u32(input)?;
        let status = input.read_u8()?;
        let x = input.read_u8()?;
        let y = input.read_u8()?;
        input.skip(1)?;
        Ok(Self {
            unit_type_id,
            status,
            location: (x, y),
        })
    }
}

/// One straight line of a perimeter wall.
#[derive(Debug, Default, Clone, Copy)]
pub struct WallLine {
    /// The kind of line. Stored from version 10.78, 0 before.
    pub line_type: u32,
    /// The wall unit type, if set.
    pub wall_type: Option<u32>,
    /// The number of gates in the line. Stored from version 10.78, 0 before.
    pub gate_count: u32,
    /// The number of wall segments in the line.
    pub segment_count: u32,
    /// The number of invisible segments. Stored from version 11.34, 0 before.
    pub invisible_segment_count: u32,
    /// The number of unfinished segments. Stored from version 11.29, 0 before.
    pub unfinished_segment_count: u32,
    /// The tile (x, y) where the line starts.
    pub line_start: (u32, u32),
    /// The tile (x, y) where the line ends.
    pub line_end: (u32, u32),
}

impl ReadableHeaderElement for WallLine {
    fn read_from<R: Read>(input: &mut RecordingHeaderReader<R>) -> Result<Self> {
        let mut line = Self::default();
        if input.version() >= 10.78 {
            line.line_type = input.read_u32::<LE>()?;
        }
        line.wall_type = read_opt_u32(input)?;
        if input.version() >= 10.78 {
            line.gate_count = input.read_u32::<LE>()?;
        }
        line.segment_count = input.read_u32::<LE>()?;
        if input.version() >= 11.34 {
            line.invisible_segment_count = input.read_u32::<LE>()?;
        }
        if input.version() >= 11.29 {
            line.unfinished_segment_count = input.read_u32::<LE>()?;
        }
        line.line_start = (input.read_u32::<LE>()?, input.read_u32::<LE>()?);
        line.line_end = (input.read_u32::<LE>()?, input.read_u32::<LE>()?);
        Ok(line)
    }
}

/// A wall the AI plans around its town.
#[derive(Debug, Default, Clone)]
pub struct PerimeterWall {
    /// Whether the AI uses this wall. Stored from version 11.22; true before.
    pub enabled: bool,
    /// The lines the wall is made of.
    pub lines: Vec<WallLine>,
    /// The number of gates in the wall. Stored from version 11.20, 0 before.
    pub gate_count: u32,
    /// The number of lines a gate fits in. The reader does not read it, so it is always 0.
    pub gate_fitting_line_count: u32,
    /// How much of the wall is built, in percent. Stored from version 11.20, 0 before.
    pub percentage_complete: u32,
    /// The number of wall segments. Stored from version 11.20, 0 before.
    pub segment_count: u32,
    /// The number of invisible segments. Stored from version 11.34, 0 before.
    pub invisible_segment_count: u32,
    /// The number of unfinished segments. Stored from version 11.20, 0 before.
    pub unfinished_segment_count: u32,
    /// The index of the line the AI updates next. Stored from version 11.29, 0 before.
    pub next_line_to_refresh: u32,
    /// The index of the segment the AI updates next. Stored from version 11.29, 0 before.
    pub next_segment_to_refresh: u32,
}

impl ReadableHeaderElement for PerimeterWall {
    fn read_from<R: Read>(input: &mut RecordingHeaderReader<R>) -> Result<Self> {
        let mut wall = Self {
            enabled: if input.version() >= 11.22 {
                input.read_u32::<LE>()? != 0
            } else {
                true
            },
            ..Default::default()
        };
        let num_lines = input.read_u32::<LE>()?;
        if input.version() >= 11.20 {
            wall.gate_count = input.read_u32::<LE>()?;
            wall.percentage_complete = input.read_u32::<LE>()?;
            wall.segment_count = input.read_u32::<LE>()?;
            if input.version() >= 11.34 {
                wall.invisible_segment_count = input.read_u32::<LE>()?;
            }
            wall.unfinished_segment_count = input.read_u32::<LE>()?;
        }
        if input.version() >= 11.29 {
            wall.next_line_to_refresh = input.read_u32::<LE>()?;
            wall.next_segment_to_refresh = input.read_u32::<LE>()?;
        }

        wall.lines = {
            let mut list = vec![];
            for _ in 0..num_lines {
                list.push(WallLine::read_from(input)?);
            }
            list
        };

        Ok(wall)
    }
}

/// What the information AI remembers about an attack.
#[derive(Debug, Default, Clone, Copy)]
pub struct AttackMemory {
    /// The attack's ID, if set.
    pub id: Option<i32>,
    /// The kind of attack.
    pub typ: u8,
    /// The smallest X tile of the attacked area.
    pub min_x: u8,
    /// The smallest Y tile of the attacked area.
    pub min_y: u8,
    /// The largest X tile of the attacked area.
    pub max_x: u8,
    /// The largest Y tile of the attacked area.
    pub max_y: u8,
    /// The attacking player, if known.
    pub attacking_owner: Option<u8>,
    /// The attacked player, if known.
    pub target_owner: Option<u8>,
    /// The number of kills in the attack.
    pub kills: u16,
    /// Whether the attack succeeded.
    pub success: bool,
    /// The game time of the attack, if set.
    pub timestamp: Option<u32>,
    /// The AI play (attack plan) used for the attack, if any.
    pub play: Option<i32>,
}

impl ReadableHeaderElement for AttackMemory {
    fn read_from<R: Read>(input: &mut RecordingHeaderReader<R>) -> Result<Self> {
        let mut mem = Self {
            id: read_opt_u32(input)?,
            typ: input.read_u8()?,
            min_x: input.read_u8()?,
            min_y: input.read_u8()?,
            max_x: input.read_u8()?,
            max_y: input.read_u8()?,
            attacking_owner: match input.read_i8()? {
                -1 => None,
                id => Some(id.try_into().unwrap()),
            },
            target_owner: match input.read_i8()? {
                -1 => None,
                id => Some(id.try_into().unwrap()),
            },
            ..Default::default()
        };
        input.skip(1)?;
        mem.kills = input.read_u16::<LE>()?;
        mem.success = input.read_u8()? != 0;
        input.skip(1)?;
        mem.timestamp = read_opt_u32(input)?;
        mem.play = read_opt_u32(input)?;
        Ok(mem)
    }
}

/// What the information AI remembers about a resource object.
#[derive(Debug, Default, Clone, Copy)]
pub struct ResourceMemory {
    /// The resource object's ID.
    pub id: ObjectID,
    /// The resource's tile position (x, y).
    pub location: (u8, u8),
    /// How often the AI tried to gather from it.
    pub gather_attempts: u8,
    /// The amount gathered from it, as the reader names it.
    pub gather: u32,
    /// Whether the entry is valid.
    pub valid: bool,
    /// Whether the resource is gone.
    pub gone: bool,
    /// The distance to the drop site.
    pub drop_distance: u8,
    /// The resource type.
    pub resource_type: u8,
    /// The ID of the drop site used for the resource.
    pub dropsite_id: ObjectID,
    /// The game time when gatherers at the resource were last attacked, if they were. Stored
    /// from version 10.91.
    pub attacked_time: Option<u32>,
}

impl ReadableHeaderElement for ResourceMemory {
    fn read_from<R: Read>(input: &mut RecordingHeaderReader<R>) -> Result<Self> {
        let mut mem = Self {
            id: input.read_u32::<LE>()?.into(),
            location: (input.read_u8()?, input.read_u8()?),
            gather_attempts: input.read_u8()?,
            gather: input.read_u32::<LE>()?,
            valid: input.read_u8()? != 0,
            gone: input.read_u8()? != 0,
            drop_distance: input.read_u8()?,
            resource_type: input.read_u8()?,
            dropsite_id: input.read_u32::<LE>()?.into(),
            ..Default::default()
        };

        if input.version() >= 10.91 {
            mem.attacked_time = read_opt_u32(input)?;
        }
        Ok(mem)
    }
}

/// An influence map: one value per tile, used by the AI for decisions such as pathing.
#[derive(Debug, Default, Clone)]
pub struct InfluenceMap {
    /// The map's width in tiles.
    pub width: u32,
    /// The map's height in tiles.
    pub height: u32,
    /// The map's reference point (x, y).
    pub reference_point: (u32, u32),
    /// The map's unchangeable limit, as the reader names it; its effect is not known yet.
    pub unchangeable_limit: u8,
    /// The values, `width * height` of them, row by row.
    pub values: Vec<i8>,
}

impl ReadableHeaderElement for InfluenceMap {
    fn read_from<R: Read>(input: &mut RecordingHeaderReader<R>) -> Result<Self> {
        let mut map = Self::default();
        map.width = input.read_u32::<LE>()?;
        map.height = input.read_u32::<LE>()?;
        map.reference_point = (input.read_u32::<LE>()?, input.read_u32::<LE>()?);
        map.unchangeable_limit = input.read_u8()?;
        map.values = vec![0; (map.width * map.height) as usize];
        input.read_i8_into(&mut map.values)?;
        Ok(map)
    }
}

/// The information AI's statistics for one quadrant of the map.
#[derive(Debug, Default, Clone, Copy)]
pub struct QuadrantLog {
    /// The number of explored tiles in the quadrant.
    pub explored_tiles: u32,
    /// The number of attacks on the AI in the quadrant.
    pub attacks_on_us: u32,
    /// The number of attacks by the AI in the quadrant.
    pub attacks_by_us: u32,
}

impl ReadableHeaderElement for QuadrantLog {
    fn read_from<R: Read>(input: &mut RecordingHeaderReader<R>) -> Result<Self> {
        let explored_tiles = input.read_u32::<LE>()?;
        let attacks_on_us = input.read_u32::<LE>()?;
        let attacks_by_us = input.read_u32::<LE>()?;
        Ok(Self {
            explored_tiles,
            attacks_on_us,
            attacks_by_us,
        })
    }
}

/// The information AI module: what the AI knows about the map, objects and other players.
#[derive(Debug, Default, Clone)]
pub struct InformationAI {
    /// Four lists of `u32` values, one per resource type; their meaning is not known yet.
    pub resource_types: Vec<Vec<u32>>,
    /// The AI's doctrine value.
    pub doctrine: i32,
    /// The AI's random number.
    pub random_number: i32,
    /// The AI's 40 goal values.
    pub goals: Vec<i32>,
    /// The map size in tiles (x, y).
    pub map_size: (u32, u32),
    /// The building lots in use (empty slots are left out).
    pub building_lots: Vec<BuildingLot>,
    /// The AI's two perimeter walls.
    pub perimeter_walls: (PerimeterWall, PerimeterWall),
    /// What the AI remembers about attacks.
    pub attack_memories: Vec<AttackMemory>,
    /// The resources the AI knows of, one list per resource type.
    pub resource_memories: [Vec<ResourceMemory>; 4],
    /// What the AI remembers about important objects (empty slots are left out).
    pub important_objects: Vec<ImportantObjectMemory>,
    /// The IDs of the important objects.
    pub important_object_ids: Vec<ObjectID>,
    /// The IDs of the important units.
    pub important_unit_ids: Vec<ObjectID>,
    /// The IDs of other important objects.
    pub important_misc_ids: Vec<ObjectID>,
    /// The IDs of the objects the AI defends.
    pub items_to_defend: Vec<ObjectID>,
    /// The IDs of the AI player's buildings.
    pub player_buildings: Vec<ObjectID>,
    /// The IDs of the AI player's objects.
    pub player_objects: Vec<ObjectID>,
    /// Object counts by unit type: 750, 850 or 900 entries, depending on the version.
    pub object_counts: Vec<u32>,
    /// The influence map the AI uses for pathing.
    pub path_map: InfluenceMap,
    /// Statistics for each of the four map quadrants.
    pub quadrant_log: [QuadrantLog; 4],
}

impl ReadableHeaderElement for InformationAI {
    // Reads the module's fields one after another, in file order.
    #[allow(clippy::cognitive_complexity, clippy::too_many_lines)]
    fn read_from<R: Read>(input: &mut RecordingHeaderReader<R>) -> Result<Self> {
        let mut ai = Self::default();
        for _ in 0..4096 {
            let _garbage = input.read_u32::<LE>()?;
        }
        let mut resource_type_counts = vec![0; 4096];
        input.read_u32_into::<LE>(&mut resource_type_counts)?;
        ai.doctrine = input.read_i32::<LE>()?;
        ai.random_number = input.read_i32::<LE>()?;
        for _ in 0..40 {
            ai.goals.push(input.read_i32::<LE>()?);
        }

        // tribute memory
        for _ in 0..8 {
            for _ in 0..4 {
                let _a = input.read_u32::<LE>()?;
                let _b = input.read_u32::<LE>()?;
            }
        }

        if ai.resource_types.len() < 4 {
            ai.resource_types.resize(4, vec![]);
        }
        for resource_id in 0..4 {
            let num_res = input.read_u32::<LE>()?;
            let list = &mut ai.resource_types[resource_id];
            for _ in 0..num_res {
                list.push(input.read_u32::<LE>()?);
            }
        }

        ai.map_size = (input.read_u32::<LE>()?, input.read_u32::<LE>()?);

        let _last_update_row = input.read_u32::<LE>()?;
        ai.important_objects = {
            let max_important_object_memory = input.read_u32::<LE>()?;
            let mut important_objects = vec![];
            for _ in 0..max_important_object_memory {
                let important_object = ImportantObjectMemory::read_from(input)?;
                if important_object.id.is_none() {
                    continue;
                }
                important_objects.push(important_object);
            }
            important_objects
        };

        ai.building_lots = {
            let len = input.read_u32::<LE>()?;
            let mut lots = vec![];
            for _ in 0..len {
                let lot = BuildingLot::read_from(input)?;
                if lot.unit_type_id.is_none() {
                    continue;
                }
                lots.push(lot);
            }
            lots
        };

        ai.perimeter_walls = (
            PerimeterWall::read_from(input)?,
            PerimeterWall::read_from(input)?,
        );

        ai.attack_memories = {
            let len = input.read_u32::<LE>()?;
            let mut attacks = vec![];
            for _ in 0..len {
                let attack = AttackMemory::read_from(input)?;
                // if attack.unit_type_id.is_none() {
                //     continue;
                // }
                attacks.push(attack);
            }
            attacks
        };

        ai.important_object_ids = read_id_list(input)?;
        ai.important_unit_ids = read_id_list(input)?;
        ai.important_misc_ids = read_id_list(input)?;
        ai.items_to_defend = read_id_list(input)?;
        ai.player_buildings = read_id_list(input)?;
        ai.player_objects = read_id_list(input)?;

        ai.object_counts = {
            let num_counts = if input.version() < 11.51 {
                750
            } else if input.version() < 11.65 {
                850
            } else {
                900
            };

            let mut object_counts = vec![0; num_counts];
            input.read_u32_into::<LE>(&mut object_counts)?;
            object_counts
        };

        let _building_count = input.read_u32::<LE>()?;

        ai.path_map = InfluenceMap::read_from(input)?;
        let _last_wall_position = (input.read_i32::<LE>()?, input.read_i32::<LE>()?);
        let _last_wall_position_2 = (input.read_i32::<LE>()?, input.read_i32::<LE>()?);

        if input.version() < 10.78 {
            input.skip(4 + 4 * 16)?;
        }

        let _save_learn_information = input.read_u32::<LE>()? != 0;
        let _learn_path = input.read_u32_length_prefixed_str()?;

        if input.version() < 11.25 {
            input.skip(0xFF)?;
        }

        ai.quadrant_log = [
            QuadrantLog::read_from(input)?,
            QuadrantLog::read_from(input)?,
            QuadrantLog::read_from(input)?,
            QuadrantLog::read_from(input)?,
        ];

        let _max_resources = [
            input.read_u32::<LE>()?,
            input.read_u32::<LE>()?,
            input.read_u32::<LE>()?,
            input.read_u32::<LE>()?,
        ];
        let num_resources = [
            input.read_u32::<LE>()?,
            input.read_u32::<LE>()?,
            input.read_u32::<LE>()?,
            input.read_u32::<LE>()?,
        ];

        ai.resource_memories = {
            let mut resources = [vec![], vec![], vec![], vec![]];
            for (list, &num) in resources.iter_mut().zip(num_resources.iter()) {
                list.reserve(num as usize);
                for _ in 0..num {
                    list.push(ResourceMemory::read_from(input)?);
                }
            }
            resources
        };

        let _dropsites_by_age = {
            let mut outer = vec![];
            for _ in 0..4 {
                let mut inner = [0; 4];
                input.read_u32_into::<LE>(&mut inner)?;
                outer.push(inner);
            }
            outer
        };
        let _closest_dropsite = [
            input.read_u32::<LE>()?,
            input.read_u32::<LE>()?,
            input.read_u32::<LE>()?,
            input.read_u32::<LE>()?,
        ];
        let _closest_dropsite_resource_id = [
            input.read_u32::<LE>()?,
            input.read_u32::<LE>()?,
            input.read_u32::<LE>()?,
            input.read_u32::<LE>()?,
        ];
        let _found_forest_tiles = input.read_u32::<LE>()?;

        if input.version() < 10.85 {
            input.skip(64_000)?;
        }

        if input.version() >= 10.90 {
            let mut relics_victory = [0; 9];
            input.read_exact(&mut relics_victory)?;
            let mut wonder_victory = [0; 9];
            input.read_exact(&mut wonder_victory)?;
        }

        if input.version() >= 10.94 {
            let should_farm = input.read_u32::<LE>()?;
            let have_seen_forage = input.read_u32::<LE>()?;
            let have_seen_gold = input.read_u32::<LE>()?;
            let have_seen_stone = input.read_u32::<LE>()?;
            dbg!(
                should_farm,
                have_seen_forage,
                have_seen_gold,
                have_seen_stone,
            );
        }
        if input.version() >= 10.95 {
            let have_seen_forest = input.read_u32::<LE>()?;
            dbg!(have_seen_forest);
        }

        if input.version() > 10.99 {
            let last_player_count_refresh_time = input.read_u32::<LE>()?;
            dbg!(last_player_count_refresh_time);
        }

        let player_unit_counts_size: usize = if input.version() >= 11.51 { 120 } else { 102 };
        let mut player_unit_counts = vec![vec![0; player_unit_counts_size]; 8];
        for unit_counts in &mut player_unit_counts {
            input.read_u32_into::<LE>(unit_counts)?;
        }

        if input.version() >= 11.09 {
            let mut player_total_building_counts = [0; 8];
            input.read_u32_into::<LE>(&mut player_total_building_counts)?;

            let mut player_real_total_building_counts = [0; 8];
            if input.version() >= 11.21 {
                input.read_u32_into::<LE>(&mut player_real_total_building_counts)?;
            }

            let mut player_total_unit_counts = [0; 8];
            input.read_u32_into::<LE>(&mut player_total_unit_counts)?;

            println!(
                "{player_total_building_counts:x?} {player_real_total_building_counts:x?} {player_total_unit_counts:x?}"
            );
        }

        todo!()
    }
}

/// The resource AI module.
#[derive(Debug, Default, Clone, Copy)]
pub struct ResourceAI {
    /// The number of resources the module tracks.
    pub num_resources: u32,
}

impl ReadableHeaderElement for ResourceAI {
    fn read_from<R: Read>(input: &mut RecordingHeaderReader<R>) -> Result<Self> {
        let num_resources = input.read_u32::<LE>()?;
        Ok(Self { num_resources })
    }
}

impl WritableHeaderElement for ResourceAI {
    fn write_to<W: Write>(&self, output: &mut W) -> Result<()> {
        output.write_u32::<LE>(self.num_resources)?;
        Ok(())
    }
}

/// The strategy AI module: the AI's victory condition, targets and rules.
#[derive(Debug, Default, Clone)]
pub struct StrategyAI {
    /// The victory condition the AI pursues now.
    pub current_victory_condition: u32,
    /// The ID of the current target.
    pub target_id: u32,
    /// The ID of a second target.
    pub second_target_id: u32,
    /// The type of the second target.
    pub second_target_type: u32,
    /// The current target point.
    pub target_point: Waypoint,
    /// A second target point.
    pub target_point_2: Waypoint,
    /// The target attribute, as the reader names it; not known yet.
    pub target_attribute: u32,
    /// The target number, as the reader names it; not known yet.
    pub target_number: u32,
    /// The time until the AI may change its victory condition.
    pub victory_condition_change_timeout: u32,
    /// The name of the AI's rule set.
    pub ruleset_name: Option<String>,
    /// The IDs of the rules for the victory condition.
    pub vc_ruleset: Vec<u32>,
    /// The IDs of the rules being executed.
    pub executing_rules: Vec<u32>,
    /// The IDs of the idle rules.
    pub idle_rules: Vec<u32>,
    /// The expert list ID. Stored from version 9.71.
    pub expert_list_id: Option<u32>,
}

impl ReadableHeaderElement for StrategyAI {
    fn read_from<R: Read>(input: &mut RecordingHeaderReader<R>) -> Result<Self> {
        Ok(Self {
            current_victory_condition: input.read_u32::<LE>()?,
            target_id: input.read_u32::<LE>()?,
            second_target_id: input.read_u32::<LE>()?,
            second_target_type: input.read_u32::<LE>()?,
            target_point: Waypoint::read_from(input)?,
            target_point_2: Waypoint::read_from(input)?,
            target_attribute: input.read_u32::<LE>()?,
            target_number: input.read_u32::<LE>()?,
            victory_condition_change_timeout: input.read_u32::<LE>()?,
            ruleset_name: input.read_u32_length_prefixed_str()?,
            vc_ruleset: read_id_list(input)?,
            executing_rules: read_id_list(input)?,
            idle_rules: read_id_list(input)?,
            expert_list_id: if input.version() >= 9.71 {
                Some(input.read_u32::<LE>()?)
            } else {
                None
            },
        })
    }
}

/// The tactical AI module: the AI's units by role, its targets and its groups.
///
/// Reading it is not finished: the reader stops after `trade_carts`, so the later fields are
/// never filled in.
#[derive(Debug, Default, Clone)]
pub struct TacticalAI {
    /// The AI's civilian units.
    pub civilians: Vec<ObjectID>,
    /// The civilians used as explorers.
    pub civilian_explorers: Vec<ObjectID>,
    /// The AI's soldiers.
    pub soldiers: Vec<ObjectID>,
    /// The soldiers that are in no group.
    pub ungrouped_soldiers: Vec<ObjectID>,
    /// The AI's boats.
    pub boats: Vec<ObjectID>,
    /// The AI's war boats.
    pub war_boats: Vec<ObjectID>,
    /// The AI's fishing boats.
    pub fishing_boats: Vec<ObjectID>,
    /// The AI's trade boats.
    pub trade_boats: Vec<ObjectID>,
    /// The AI's transport boats.
    pub transport_boats: Vec<ObjectID>,
    /// The artifacts (such as relics) the AI knows of.
    pub artifacts: Vec<ObjectID>,
    /// The AI's trade carts.
    pub trade_carts: Vec<ObjectID>,
    /// The AI's strategic numbers. Not read yet.
    pub strategic_numbers: Vec<i32>,
    /// The players the AI attacks. Not read yet.
    pub players_to_attack: Vec<u32>,
    /// The players the AI defends. Not read yet.
    pub players_to_defend: Vec<u32>,
    /// The AI's working area. Not read yet.
    pub working_area: Vec<u32>,
    /// The units the AI gave tasks in this update. Not read yet.
    pub units_tasked_this_update: Vec<ObjectID>,
    /// How the AI's gatherers are spread over the four resources. Not read yet.
    pub gatherer_distribution: [u32; 4],
    /// How the AI wants its gatherers spread over the four resources. Not read yet.
    pub desired_gatherer_distribution: [u32; 4],
    /// The amount of each of the four resources the AI needs. Not read yet.
    pub resources_needed: [u32; 4],
    /// The AI's group ID. Not read yet.
    pub group_id: u32,
    /// The AI's unit groups. Not read yet.
    pub groups: Vec<()>,
}

impl ReadableHeaderElement for TacticalAI {
    fn read_from<R: Read>(input: &mut RecordingHeaderReader<R>) -> Result<Self> {
        let mut ai = Self {
            civilians: read_id_list(input)?,
            civilian_explorers: read_id_list(input)?,
            ..Default::default()
        };

        let _num_gatherers = input.read_u32::<LE>()?;
        let _desired_num_gatherers = input.read_u32::<LE>()?;

        // FIXME: more stuff here

        ai.soldiers = read_id_list(input)?;
        ai.ungrouped_soldiers = read_id_list(input)?;
        ai.boats = read_id_list(input)?;
        ai.war_boats = read_id_list(input)?;
        ai.fishing_boats = read_id_list(input)?;
        ai.trade_boats = read_id_list(input)?;
        ai.transport_boats = read_id_list(input)?;
        ai.artifacts = read_id_list(input)?;
        ai.trade_carts = read_id_list(input)?;

        todo!()
    }
}

/// The AI state of one computer player: its AI modules in file order.
#[derive(Debug, Default, Clone)]
pub struct PlayerAI {
    /// The main AI module.
    pub main_ai: MainAI,
    /// The build AI module.
    pub build_ai: BuildAI,
    /// The construction AI module.
    pub construction_ai: ConstructionAI,
    /// The diplomacy AI module.
    pub diplomacy_ai: DiplomacyAI,
    /// The emotional AI module.
    pub emotional_ai: EmotionalAI,
    /// The information AI module.
    pub information_ai: InformationAI,
    /// The resource AI module.
    pub resource_ai: ResourceAI,
    /// The strategy AI module.
    pub strategy_ai: StrategyAI,
}

impl ReadableHeaderElement for PlayerAI {
    fn read_from<R: Read>(input: &mut RecordingHeaderReader<R>) -> Result<Self> {
        let main_ai = MainAI::read_from(input)?;
        let build_ai = BuildAI::read_from(input)?;
        let construction_ai = ConstructionAI::read_from(input)?;
        let diplomacy_ai = DiplomacyAI::read_from(input)?;
        let emotional_ai = EmotionalAI::read_from(input)?;
        let information_ai = InformationAI::read_from(input)?;
        let resource_ai = ResourceAI::read_from(input)?;
        let strategy_ai = StrategyAI::read_from(input)?;

        Ok(Self {
            main_ai,
            build_ai,
            construction_ai,
            diplomacy_ai,
            emotional_ai,
            information_ai,
            resource_ai,
            strategy_ai,
        })
    }
}

fn read_id_list<T: From<u32>, R: Read>(input: &mut R) -> Result<Vec<T>> {
    let len = input.read_u32::<LE>()?;
    let mut ids = vec![];
    for _ in 0..len {
        ids.push(input.read_u32::<LE>()?.into());
    }
    Ok(ids)
}
