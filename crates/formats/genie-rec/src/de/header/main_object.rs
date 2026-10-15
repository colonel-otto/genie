//! The per-player main objects (pattern: `InitObject` and its variants).

use super::primitives::DeString;
use binrw::{BinRead, binread};

/// One main object (unit type) of a player, as the world save stores it.
#[binread]
#[derive(Debug, Clone, PartialEq)]
#[br(little, import(v: u32))]
pub struct MainObject {
    #[br(magic = b"\x16")]
    pub kind: u8,
    pub id: [u16; 4],
    pub class: i16,
    pub unknown1: [u8; 2],
    pub hit_points: u16,
    pub line_of_sight: f32,
    pub garrison_capacity: u8,
    pub collision_size_x: f32,
    pub collision_size_y: f32,
    pub resource_capacity: i16,
    pub resources: [ResourceStorage; 3],
    /// 1 when the unit is disabled for the player (158041: `Write(&disabled, 1)` at `0x141139adb`
    /// in the main static object save `0x1411380d0`). The pattern reads it as a magic `0x00`.
    pub disabled: u8,
    pub name_string_id: u32,
    pub create_string_id: u32,
    pub terrain_table: i16,
    pub dead_unit: i16,
    pub icon: i16,
    pub unknown3: [u8; 9],
    pub collision_size_z: f32,
    pub obstruction_type: u8,
    pub blockage_class: u8,
    pub unknown5: u8,
    pub standing_graphics: [i16; 2],
    pub dying_graphics: [i16; 2],
    #[br(magic = b"\xfe\xff\xff\xff")]
    pub unknown7: u8,
    pub unknown8: u32,
    pub sounds: [i16; 4],
    pub sounds_big: [u32; 4],
    pub interface_kind: i8,
    pub combat_level: i8,
    pub interaction_mode: i8,
    pub minimap_mode: i8,
    pub unknown9: i8,
    #[br(temp)]
    damage_graphic_count: i8,
    #[br(count = damage_graphic_count.max(0) as usize)]
    pub damage_graphics: Vec<DamageGraphic>,
    pub internal_name: DeString,
    #[br(parse_with = main_body, args(kind, v))]
    pub body: MainBody,
    #[br(magic = b"\x21", temp)]
    end: (),
}

#[binread]
#[derive(Debug, Clone, Copy, PartialEq)]
#[br(little)]
pub struct ResourceStorage {
    pub kind: i16,
    pub amount: f32,
    pub store_mode: u8,
}

#[binread]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[br(little)]
pub struct DamageGraphic {
    pub graphic_id: i16,
    pub damage_percent: u8,
    pub apply_mode: u8,
}

/// The type-specific part of a main object.
#[derive(Debug, Clone, PartialEq)]
pub enum MainBody {
    /// Type 10.
    Static,
    /// Types 20 and 25.
    Animated(u32),
    /// Type 30.
    Moving(MainMoving),
    /// Type 60.
    Missile(MainMissile),
    /// Type 70.
    Combat(MainCombat),
    /// Type 80.
    Building(MainBuilding),
}

/// Dispatch on the type byte by hand (no variant is tried and rejected).
#[binrw::parser(reader, endian)]
fn main_body(kind: u8, v: u32) -> binrw::BinResult<MainBody> {
    Ok(match kind {
        10 => MainBody::Static,
        20 | 25 => MainBody::Animated(u32::read_options(reader, endian, ())?),
        30 => MainBody::Moving(MainMoving::read_options(reader, endian, (v,))?),
        60 => MainBody::Missile(MainMissile::read_options(reader, endian, (v,))?),
        70 => MainBody::Combat(MainCombat::read_options(reader, endian, (v,))?),
        80 => MainBody::Building(MainBuilding::read_options(reader, endian, (v,))?),
        other => {
            return Err(binrw::Error::AssertFail {
                pos: reader.stream_position()?,
                message: format!("unknown main object type {other}"),
            });
        }
    })
}

#[binread]
#[derive(Debug, Clone, PartialEq)]
#[br(little, import(v: u32))]
pub struct MainMoving {
    pub x: u32,
    pub y: u32,
    pub z: u32,
    pub unknown1: [u8; 7],
    #[br(if(v >= 6720))]
    pub unknown_67_2: Option<u8>,
}

/// The offence part shared by missiles, combat units and buildings (pattern: `InitBaseOffensive`).
#[binread]
#[derive(Debug, Clone, PartialEq)]
#[br(little, import(v: u32))]
pub struct MainOffence {
    pub speed: f32,
    pub rotation_speed: f32,
    pub running_graphics: [i16; 2],
    pub unknown_ibo_23: [u8; 7],
    #[br(if(v >= 6720))]
    pub unknown_67_2: Option<u8>,
    pub line_of_sight: f32,
    pub work_rate: f32,
    /// `mHandicappedWorkRate` (158041: the small action-object save `0x1410a4e70`).
    pub unknown5: f32,
    /// `mObjectTaskFlags`. With bit 0 set, the player's own task list follows (a civilisation's
    /// villagers or fortified church, for example).
    pub object_task_flags: u8,
    #[br(if(object_task_flags & 1 != 0))]
    pub tasks: Option<TaskList>,
    pub attack_sound: i16,
    pub move_sound: i16,
    pub attack_sound_big: u32,
    pub move_sound_big: u32,
    pub unknown29: u8,
    pub base_armour: u16,
    #[br(temp)]
    attack_bonus_count: u16,
    #[br(count = attack_bonus_count)]
    pub attack_bonuses: Vec<ClassAmount>,
    #[br(temp)]
    armor_count: u16,
    #[br(count = armor_count)]
    pub armor_classes: Vec<ClassAmount>,
    pub reload_time: f32,
    pub max_range: f32,
    pub accuracy: i16,
    pub projectile_unit: i16,
    pub terrain_defense_bonus: i16,
    pub bonus_damage_resist: f32,
    pub blast_width: f32,
    pub min_range: f32,
    pub frame_delay: i16,
    pub blast_attack_level: u8,
    pub melee_armor: i16,
    pub attack: i16,
    pub displayed_range: f32,
    pub break_off_combat: u8,
    pub attack_dispersion: f32,
    pub attack_graphic: i16,
    pub blast_damage: f32,
    pub unknown15: f32,
    pub friendly_fire: f32,
    pub interrupt_frame: i16,
    pub garrison_firepower: f32,
    pub attack_graphic_2: i16,
    pub graphic_displacement: [f32; 3],
}

/// A main object's own task list (`tasks->save`, `0x1411a9720`).
#[binread]
#[derive(Debug, Clone, PartialEq)]
#[br(little)]
pub struct TaskList {
    #[br(temp)]
    count: i16,
    #[br(count = count.max(0) as usize)]
    pub tasks: Vec<Task>,
}

/// One task (`0x1410652a0`), in write order.
#[binread]
#[derive(Debug, Clone, Copy, PartialEq)]
#[br(little)]
pub struct Task {
    pub task_type: i16,
    pub id: i16,
    pub is_default: u8,
    pub action_type: i16,
    pub object_group: i16,
    pub object_id: i16,
    pub terrain_id: i16,
    pub attribute_types: [i16; 4],
    pub work_value_1: f32,
    pub work_value_2: f32,
    pub work_range: f32,
    pub search_flag: u8,
    pub search_wait_time: f32,
    pub combat_level: u8,
    pub combat_level_flag: u8,
    pub work_flag: i16,
    pub work_flag_2: i16,
    pub owner_type: u8,
    pub holding_attribute: u8,
    pub state_building: u8,
    /// Six values the binary writes through one `short_val` temporary.
    pub short_values: [i16; 6],
    pub gather_sound_event: u32,
    pub deposit_sound_event: u32,
    pub enablement_resource_id: i16,
}

/// An attack bonus or armour entry: an armour class and its amount.
#[binread]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[br(little)]
pub struct ClassAmount {
    pub class: u16,
    pub amount: i16,
}

#[binread]
#[derive(Debug, Clone, PartialEq)]
#[br(little, import(v: u32))]
pub struct MainMissile {
    #[br(args(v))]
    pub offence: MainOffence,
    pub unknown20: [u8; 3],
    pub unknown21: f32,
}

#[binread]
#[derive(Debug, Clone, PartialEq)]
#[br(little, import(v: u32))]
pub struct MainCombat {
    #[br(args(v))]
    pub offence: MainOffence,
    pub cost: [UnitCost; 3],
    #[br(temp)]
    train_locations_count: u16,
    #[br(count = train_locations_count)]
    pub train_locations: Vec<TrainLocation>,
    pub total_projectiles: f32,
    pub max_total_projectiles: u8,
    pub unknown23: u8,
    pub unknown26: u8,
    pub maybe_displayed_armor: u16,
    pub unknown27: [u8; 3],
    pub max_charge: f32,
    pub recharge_rate: f32,
    pub charge_event: i16,
    pub charge_type: i16,
    pub charge_target: i16,
    pub charge_projectile_unit: i32,
    pub unknown28_1: [u8; 13],
    pub unknown28: [u8; 4],
    pub min_conversion_time: f32,
    pub max_conversion_time: f32,
    pub conversion_chance: f32,
    pub unknown31: [u8; 7],
    pub unknown34: u8,
    pub idle_attack_graphic: i16,
    pub hero_glow_graphic: i16,
    pub unknown36: i16,
    pub projectile_spawning_area: [f32; 3],
    #[br(if(v >= 6660))]
    pub unknown39: Option<[u8; 2]>,
}

#[binread]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[br(little)]
pub struct UnitCost {
    pub kind: i16,
    pub cost: i16,
    pub deduct: i16,
}

#[binread]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[br(little)]
pub struct TrainLocation {
    pub train_time: i16,
    pub location_id: i16,
    pub button_id: u8,
    pub hotkey_id: u32,
}

#[binread]
#[derive(Debug, Clone, PartialEq)]
#[br(little, import(v: u32))]
pub struct MainBuilding {
    #[br(args(v))]
    pub combat: MainCombat,
    pub unknown24: [u8; 2],
    pub garrison_heal_rate: f32,
    pub unknown25: [i16; 2],
    pub garrison_type: u8,
    pub unknown30: [u8; 2],
    pub unknown_building_1: [u8; 24],
}
