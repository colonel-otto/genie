//! A unit type saved in full: master vtable slot 7, the `.dat` unit layout.
//!
//! The world save writes one when an object has its own master (`hasOwnMasterObject`) or a successor
//! master (`hasSuccessorMasterObj`), and for a gate's open/closed master. The player's master list
//! uses the shorter slot-6 save instead ([`MainObject`](super::MainObject)).

use super::main_object::{ClassAmount, ResourceStorage, TaskList, TrainLocation, UnitCost};
use super::primitives::DeString;
use binrw::{BinRead, binread};

/// One master in write order (158041: `RGE_Master_Static_Object` slot 7, `0x141127760`). Names are
/// the binary's; its `readTemp` temporaries are named after the field they carry.
#[binread]
#[derive(Debug, Clone, PartialEq)]
#[br(little)]
pub struct MasterObject {
    pub master_type: u8,
    pub id: i16,
    pub string_id: i32,
    pub string_id2: i32,
    pub object_group: i16,
    pub sprite: i16,
    pub sprite2: i16,
    pub death_sprite: i16,
    pub undead_sprite: i16,
    pub undead_flag: u8,
    pub hp: i16,
    pub los: f32,
    pub obj_max: u8,
    pub radius: [f32; 3],
    pub created_sound: i16,
    pub damage_sound: i16,
    pub death_spawn_obj_id: i16,
    pub death_spawn_obj_id2: i16,
    pub sort_number: u8,
    pub can_be_built_on: u8,
    pub button_pict: i16,
    pub hide_in_scenario_editor: u8,
    pub portrait_pict: i16,
    pub available: u8,
    pub disabled: u8,
    pub tile_req: [i16; 2],
    pub center_tile_req: [i16; 2],
    pub construction_radius: [f32; 2],
    pub elevation_flag: u8,
    pub fog_flag: u8,
    pub terrain_restriction_mask_id: i16,
    pub movement_type: u8,
    pub attribute_max_amount: i16,
    pub attribute_rot: f32,
    pub area_effect_object_level: u8,
    pub combat_level: u8,
    pub select_level: u8,
    pub map_draw_level: u8,
    pub unit_level: u8,
    pub multiple_attribute_mod: f32,
    pub map_color: u8,
    pub help_string_id: i32,
    pub help_page_id: i32,
    pub recyclable: u8,
    pub track_as_resource: u8,
    pub create_doppleganger_on_death: u8,
    pub resource_group: u8,
    pub occlusion_flag: u8,
    pub obstruction_type: u8,
    pub obstruction_class: u8,
    pub object_flags: u32,
    pub draw_flag: u8,
    pub draw_color: u8,
    pub outline_radius: [f32; 3],
    pub override_description: OverrideDescription,
    /// Type, amount and flag of each attribute held.
    pub attributes: [ResourceStorage; 3],
    #[br(temp)]
    damage_sprite_count: u8,
    #[br(count = damage_sprite_count)]
    pub damage_sprites: Vec<DamageSprite>,
    pub selected_sound: i16,
    pub death_sound: i16,
    pub created_sound_event: u32,
    pub damage_sound_event: u32,
    pub selected_sound_event: u32,
    pub death_sound_event: u32,
    pub attack_reaction: u8,
    pub convert_terrain_flag: u8,
    pub name: DeString,
    pub copy_id: i16,
    pub base_id: i16,
    #[br(parse_with = master_body, args(master_type))]
    pub body: MasterBody,
}

/// `WriteOverrideDescriptionString` (`0x141120d00`): a string id, then a string whose i32 size
/// includes its null terminator.
#[binread]
#[derive(Debug, Clone, PartialEq, Eq)]
#[br(little)]
pub struct OverrideDescription {
    pub string_id: i32,
    #[br(temp)]
    size: i32,
    #[br(count = size.max(0) as usize)]
    pub text: Vec<u8>,
}

/// A damage sprite: the sprite, the damage percent that shows it, a padding byte, the flag.
#[binread]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[br(little)]
pub struct DamageSprite {
    pub sprite: i16,
    pub damage_percent: u8,
    pub padding: u8,
    pub flag: u8,
}

/// The parts each master type adds, every type containing the previous one's.
#[derive(Debug, Clone, PartialEq)]
pub enum MasterBody {
    /// Types 10 and 90.
    Static,
    /// Types 20 and 25.
    Animated { speed: f32 },
    /// Type 30.
    Moving { speed: f32, moving: MasterMoving },
    /// Type 40.
    Action {
        speed: f32,
        moving: MasterMoving,
        action: MasterAction,
    },
    /// Type 50.
    Combat {
        speed: f32,
        moving: MasterMoving,
        action: MasterAction,
        combat: MasterCombat,
    },
    /// Type 60.
    Missile {
        speed: f32,
        moving: MasterMoving,
        action: MasterAction,
        combat: MasterCombat,
        missile: MasterMissile,
    },
    /// Type 70.
    Creatable {
        speed: f32,
        moving: MasterMoving,
        action: MasterAction,
        combat: MasterCombat,
        creatable: MasterCreatable,
    },
    /// Type 80.
    Building {
        speed: f32,
        moving: MasterMoving,
        action: MasterAction,
        combat: MasterCombat,
        creatable: MasterCreatable,
        building: MasterBuilding,
    },
}

/// `RGE_Master_Moving_Object` slot 7 (`0x1410e1ff0`).
#[binread]
#[derive(Debug, Clone, Copy, PartialEq)]
#[br(little)]
pub struct MasterMoving {
    pub move_sprite: i16,
    pub run_sprite: i16,
    pub turn_speed: f32,
    pub size_class: u8,
    pub obj_trail_id: i16,
    pub obj_trail_options: u8,
    pub obj_trail_spacing: f32,
    pub move_algorithm: u8,
    pub turn_radius: f32,
    pub turn_radius_speed: f32,
    pub max_yaw_per_second_moving: f32,
    pub stationary_yaw_revolution_time: f32,
    pub max_yaw_per_second_stationary: f32,
    pub friendly_collision_circle_scale: f32,
}

/// `RGE_Master_Action_Object` slot 7 (`0x14109ff30`).
#[binread]
#[derive(Debug, Clone, PartialEq)]
#[br(little)]
pub struct MasterAction {
    pub default_task: i16,
    pub search_radius: f32,
    pub work_rate: f32,
    #[br(temp)]
    drop_site_count: i16,
    #[br(count = drop_site_count.max(0) as usize)]
    pub drop_sites: Vec<i16>,
    pub task_by_group: u8,
    pub command_sound: i16,
    pub move_sound: i16,
    pub command_sound_event: u32,
    pub move_sound_event: u32,
    pub run_pattern: u8,
    pub tasks: TaskList,
}

/// `RGE_Master_Combat_Object` slot 7 (`0x1410c2190`).
#[binread]
#[derive(Debug, Clone, PartialEq)]
#[br(little)]
pub struct MasterCombat {
    pub base_armor: i16,
    #[br(temp)]
    weapon_count: i16,
    #[br(count = weapon_count.max(0) as usize)]
    pub weapons: Vec<ClassAmount>,
    #[br(temp)]
    armor_count: i16,
    #[br(count = armor_count.max(0) as usize)]
    pub armors: Vec<ClassAmount>,
    pub defense_terrain_bonus: i16,
    pub bonus_damage_resistance: f32,
    pub weapon_range: f32,
    pub area_effect_range: f32,
    pub speed_of_attack: f32,
    pub missile_id: i16,
    pub base_hit_chance: i16,
    pub combat_ability: u8,
    pub fire_missile_at_frame: i16,
    pub weapon_offset: [f32; 3],
    pub area_effect_level: u8,
    pub minimum_weapon_range: f32,
    pub missed_missile_spread: f32,
    pub fight_sprite: i16,
    pub orig_armor: i16,
    pub orig_weapon: i16,
    pub orig_weapon_range: f32,
    pub orig_speed_of_attack: f32,
    pub area_damage: f32,
    pub damage_reflection: f32,
    pub friendly_fire_damage: f32,
    pub interrupt_frame: i16,
    pub garrison_firepower: f32,
    pub fight_sprite2: i16,
}

/// `RGE_Master_Missile_Object` slot 7 (`0x1410d5bd0`): five bytes at `+0x880`, then `+0x888`. The
/// binary names four of the bytes; `smart_mode` is genieutils-py's name for the unnamed second.
#[binread]
#[derive(Debug, Clone, Copy, PartialEq)]
#[br(little)]
pub struct MasterMissile {
    pub targetting_type: u8,
    pub smart_mode: u8,
    pub missile_hit_info: u8,
    pub missile_die_info: u8,
    pub area_effect_specials: u8,
    pub ballistics_ratio: f32,
}

/// `TRIBE_Master_Combat_Object` slot 7 (`0x141267460`).
#[binread]
#[derive(Debug, Clone, PartialEq)]
#[br(little)]
pub struct MasterCreatable {
    pub build_inventory: [UnitCost; 3],
    #[br(temp)]
    train_location_count: i16,
    #[br(count = train_location_count.max(0) as usize)]
    pub train_locations: Vec<TrainLocation>,
    pub regeneration_rate: f32,
    pub spacing_modifier: f32,
    pub formation_category: u8,
    pub hero_flag: u8,
    pub garrison_sprite: i32,
    pub spawning_sprite: i16,
    pub upgrading_sprite: i16,
    pub hero_glow_sprite: i16,
    pub idle_attack_sprite: i16,
    pub max_charge: f32,
    pub recharge_rate: f32,
    pub charge_event: i16,
    pub charge_type: i16,
    pub charge_target: i16,
    pub charge_projectile_unit: i32,
    pub attack_priority: u8,
    pub invulnerability_level: f32,
    pub button_icon_id: i16,
    pub button_short_tooltip_id: i32,
    pub button_extended_tooltip_id: i32,
    pub button_hotkey_action: i16,
    pub min_conversion_time_adj: f32,
    pub max_conversion_time_adj: f32,
    pub conversion_chance_adj: f32,
    pub volley_fire_amount: f32,
    pub max_attacks_in_volley: u8,
    pub volley_spread: [f32; 2],
    pub volley_start_spread_adjustment: f32,
    pub volley_missile_id: i32,
    pub special_attack_sprite_id: i32,
    pub special_attack_flag: u8,
    pub orig_pierce_armor: i16,
}

/// `TRIBE_Master_Building_Object` slot 7 (`0x14123d7b0`).
#[binread]
#[derive(Debug, Clone, PartialEq)]
#[br(little)]
pub struct MasterBuilding {
    pub construction_sprite: i16,
    pub snow_sprite: i16,
    pub destruction_sprite: i16,
    pub destruction_rubble_sprite: i16,
    pub researching_sprite: i16,
    pub research_completed_sprite: i16,
    pub building_connect_flag: u8,
    pub building_facet: i16,
    pub build_and_go_away: u8,
    pub on_build_make_obj: i16,
    pub on_build_make_tile: i16,
    pub on_build_make_overlay: i16,
    pub on_build_make_tech: i16,
    pub can_burn: u8,
    /// Id and x/y offset of each linked building.
    pub linked: [(i16, f32, f32); 4],
    pub construction_id: i16,
    pub open_close_id: i16,
    pub open_close_sound: i16,
    pub sound: i16,
    pub open_close_sound_event: u32,
    pub construction_sound_event: u32,
    pub garrison_units: u8,
    pub garrison_heal_rate: f32,
    pub garrison_repair_rate: f32,
    pub salvage_id: i16,
    pub pillage_attr: [u8; 6],
}

/// Dispatch on the type byte by hand (no variant is tried and rejected).
#[binrw::parser(reader, endian)]
fn master_body(master_type: u8) -> binrw::BinResult<MasterBody> {
    if matches!(master_type, 10 | 90) {
        return Ok(MasterBody::Static);
    }
    if !matches!(master_type, 20 | 25 | 30 | 40 | 50 | 60 | 70 | 80) {
        return Err(binrw::Error::AssertFail {
            pos: reader.stream_position()?,
            message: format!("unknown master type {master_type}"),
        });
    }
    let speed = f32::read_options(reader, endian, ())?;
    if master_type < 30 {
        return Ok(MasterBody::Animated { speed });
    }
    let moving = MasterMoving::read_options(reader, endian, ())?;
    if master_type == 30 {
        return Ok(MasterBody::Moving { speed, moving });
    }
    let action = MasterAction::read_options(reader, endian, ())?;
    if master_type == 40 {
        return Ok(MasterBody::Action {
            speed,
            moving,
            action,
        });
    }
    let combat = MasterCombat::read_options(reader, endian, ())?;
    Ok(match master_type {
        50 => MasterBody::Combat {
            speed,
            moving,
            action,
            combat,
        },
        60 => MasterBody::Missile {
            speed,
            moving,
            action,
            combat,
            missile: MasterMissile::read_options(reader, endian, ())?,
        },
        70 => MasterBody::Creatable {
            speed,
            moving,
            action,
            combat,
            creatable: MasterCreatable::read_options(reader, endian, ())?,
        },
        _ => {
            let creatable = MasterCreatable::read_options(reader, endian, ())?;
            MasterBody::Building {
                speed,
                moving,
                action,
                combat,
                creatable,
                building: MasterBuilding::read_options(reader, endian, ())?,
            }
        }
    })
}

#[cfg(test)]
pub(super) mod tests {
    use super::super::test_bytes::Bytes;
    use super::*;
    use binrw::io::Cursor;

    /// The static part of a master of `master_type` and `id`, named `name`.
    pub(in super::super) fn static_part(master_type: u8, id: i16, name: &[u8]) -> Bytes {
        Bytes::default()
            .u8(master_type)
            .i16(id)
            .i32(5506) // string_id
            .i32(6506) // string_id2
            .i16(30) // object_group
            .i16(3268) // sprite
            .i16(-1) // sprite2
            .i16(-1) // death_sprite
            .i16(-1) // undead_sprite
            .u8(0) // undead_flag
            .i16(25) // hp
            .f32(2.0) // los
            .u8(0) // obj_max
            .f32(0.1) // radius x, y, z
            .f32(0.1)
            .f32(0.0)
            .zeros(8) // created and damage sounds, death spawn ids
            .u8(5) // sort_number
            .zeros(1 + 2 + 1 + 2 + 1 + 1) // can_be_built_on … disabled
            .zeros(8 + 8) // tile requirements, construction radius
            .zeros(1 + 1 + 2 + 1 + 2 + 4) // elevation flag … attribute_rot
            .zeros(5 + 4 + 1) // levels, multiple_attribute_mod, map_color
            .i32(105506) // help_string_id
            .i32(155506) // help_page_id
            .zeros(7) // recyclable … obstruction_class
            .i32(0) // object_flags
            .zeros(2 + 12) // draw flag and colour, outline radius
            .i32(-2) // override description: string id, then a sized string with its null
            .i32(1)
            .u8(0)
            .i16(-1) // three attributes: type, amount, flag
            .f32(1.0)
            .u8(0)
            .i16(-1)
            .f32(0.0)
            .u8(0)
            .i16(-1)
            .f32(0.0)
            .u8(0)
            .u8(1) // one damage sprite: id, percent, a padding byte, flag
            .i16(400)
            .u8(50)
            .u8(0)
            .u8(1)
            .i16(258) // selected sound
            .i16(-1) // death sound
            .zeros(16) // four sound events
            .zeros(2) // attack_reaction, convert_terrain_flag
            .de_string(name)
            .i16(id) // copy_id
            .i16(id) // base_id
    }

    fn read(bytes: &Bytes) -> (MasterObject, u64) {
        let mut r = Cursor::new(&bytes.0[..]);
        let master = MasterObject::read_le(&mut r).unwrap();
        (master, r.position())
    }

    #[test]
    fn an_animated_master_reads_its_static_part_and_speed() {
        let bytes = static_part(20, 600, b"FLAGA").f32(1.5);
        let (master, end) = read(&bytes);
        assert_eq!(end, bytes.0.len() as u64);
        assert_eq!(
            (
                master.master_type,
                master.id,
                master.hp,
                master.name.bytes.as_slice()
            ),
            (20, 600, 25, &b"FLAGA"[..])
        );
        assert_eq!(master.override_description.string_id, -2);
        assert_eq!(master.attributes[0].amount, 1.0);
        assert_eq!(master.damage_sprites[0].damage_percent, 50);
        assert_eq!(master.selected_sound, 258);
        assert_eq!(master.body, MasterBody::Animated { speed: 1.5 });
    }

    #[test]
    fn a_building_master_reads_every_part_down_the_chain() {
        let task = Bytes::default().i16(1).i16(7).zeros(69 - 4);
        let bytes = static_part(80, 2172, b"Port 1")
            .f32(0.0) // speed
            .zeros(2 + 2 + 4 + 1 + 2 + 1 + 4 + 1 + 6 * 4) // moving
            .i16(-1) // action: default_task, search_radius, work_rate
            .f32(0.0)
            .f32(0.0)
            .i16(2) // two drop sites
            .i16(-1)
            .i16(-1)
            .u8(0) // task_by_group
            .zeros(2 + 2 + 4 + 4 + 1) // sounds, events, run_pattern
            .i16(1) // one task
            .bytes(&task.0)
            .i16(0) // combat: base_armor
            .i16(1) // one weapon
            .i16(4)
            .i16(6)
            .i16(0) // no armour
            .i16(0) // defense_terrain_bonus
            .zeros(
                4 * 4 + 2 + 2 + 1 + 2 + 12 + 1 + 4 + 4 + 2 + 2 + 2 + 4 + 4 + 4 + 4 + 4 + 2 + 4 + 2,
            )
            .zeros(3 * 6) // creatable: three costs
            .i16(1) // one train location: time, unit id, button, hotkey
            .i16(35)
            .i16(118)
            .u8(5)
            .i32(416001)
            .zeros(4 + 4 + 1 + 1 + 4 + 2 * 4 + 4 + 4 + 2 * 3 + 4 + 1 + 4 + 2 + 4 + 4 + 2)
            .zeros(4 * 3 + 4 + 1 + 4 * 3 + 4 + 4 + 1 + 2)
            .i16(4248) // building: construction sprite
            .zeros(2 * 5 + 1 + 2 + 1 + 2 * 4)
            .u8(1) // can_burn
            .zeros(4 * 10) // four linked buildings
            .zeros(2 * 4 + 4 + 4 + 1 + 4 + 4 + 2 + 6);
        let (master, end) = read(&bytes);
        assert_eq!(end, bytes.0.len() as u64);
        let MasterBody::Building {
            action,
            combat,
            creatable,
            building,
            ..
        } = &master.body
        else {
            panic!("not a building: {:?}", master.body);
        };
        assert_eq!(action.drop_sites, vec![-1, -1]);
        assert_eq!(action.tasks.tasks[0].id, 7);
        assert_eq!(combat.weapons[0].amount, 6);
        assert_eq!(creatable.train_locations[0].hotkey_id, 416001);
        assert_eq!((building.construction_sprite, building.can_burn), (4248, 1));
    }

    #[test]
    fn a_missile_master_reads_five_bytes_and_the_ballistics_ratio() {
        let bytes = static_part(60, 9, b"ARROW")
            .f32(0.0)
            .zeros(2 + 2 + 4 + 1 + 2 + 1 + 4 + 1 + 6 * 4)
            .i16(-1)
            .zeros(8)
            .i16(0) // no drop sites
            .zeros(1 + 2 + 2 + 4 + 4 + 1)
            .i16(0) // no tasks
            .i16(0)
            .i16(0)
            .i16(0)
            .i16(0)
            .zeros(
                4 * 4 + 2 + 2 + 1 + 2 + 12 + 1 + 4 + 4 + 2 + 2 + 2 + 4 + 4 + 4 + 4 + 4 + 2 + 4 + 2,
            )
            .u8(1)
            .u8(2)
            .u8(3)
            .u8(4)
            .u8(5)
            .f32(0.5);
        let (master, end) = read(&bytes);
        assert_eq!(end, bytes.0.len() as u64);
        let MasterBody::Missile { missile, .. } = &master.body else {
            panic!("not a missile: {:?}", master.body);
        };
        assert_eq!(missile.ballistics_ratio, 0.5);
        assert_eq!(missile.area_effect_specials, 5);
    }

    #[test]
    fn an_unknown_master_type_names_the_type() {
        let bytes = static_part(15, 1, b"X");
        let err = MasterObject::read_le(&mut Cursor::new(&bytes.0[..])).unwrap_err();
        assert!(err.to_string().contains("master type 15"), "{err}");
    }
}
