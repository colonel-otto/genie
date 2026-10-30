//! The attributes an effect changes on a master and on its objects (the game's attribute list:
//! string 12200 plus the number names each).

/// "Hit Points" (`+0x530`).
pub const HIT_POINTS: u8 = 0;
/// "Line of Sight" (`+0x5d0`).
pub const LINE_OF_SIGHT: u8 = 1;
/// "Garrison Capacity" (`+0x5d4`).
pub const GARRISON_CAPACITY: u8 = 2;
/// "Unit Size X" (`+0x5d8`).
pub const SIZE_X: u8 = 3;
/// "Unit Size Y" (`+0x5dc`).
pub const SIZE_Y: u8 = 4;
/// "Movement Speed" (`+0x740`).
pub const MOVEMENT_SPEED: u8 = 5;
/// "Rotation Speed" (`+0x758`).
pub const ROTATION_SPEED: u8 = 6;
/// "Armor" (`+0x808`'s entries).
pub const ARMOR: u8 = 8;
/// "Attack" (`+0x818`'s entries).
pub const ATTACK: u8 = 9;
/// "Attack Reload Time" (`+0x838`).
pub const ATTACK_RELOAD_TIME: u8 = 10;
/// "Accuracy Percent" (`+0x83e`).
pub const ACCURACY_PERCENT: u8 = 11;
/// "Maximum Range" (`+0x828`).
pub const MAXIMUM_RANGE: u8 = 12;
/// "Work Rate" (`+0x798`).
pub const WORK_RATE: u8 = 13;
/// "Carry Capacity" (`+0x61c`).
pub const CARRY_CAPACITY: u8 = 14;
/// "Base Armor" (`+0x800`).
pub const BASE_ARMOR: u8 = 15;
/// "Projectile Unit" (`+0x83c`).
pub const PROJECTILE_UNIT: u8 = 16;
/// "Building Icon Override" (`+0x982`).
pub const BUILDING_ICON_OVERRIDE: u8 = 17;
/// "Terrain Defense Bonus" (`+0x820`).
pub const TERRAIN_DEFENSE_BONUS: u8 = 18;
/// "Projectile Smart Mode" (`+0x881`).
pub const PROJECTILE_SMART_MODE: u8 = 19;
/// "Minimum Range" (`+0x850`).
pub const MINIMUM_RANGE: u8 = 20;
/// "Amount of 1st Resource Storage" (`+0x610`).
pub const STORAGE_1: u8 = 21;
/// "Blast Width" (`+0x82c`).
pub const BLAST_WIDTH: u8 = 22;
/// "Search Radius" (`+0x794`).
pub const SEARCH_RADIUS: u8 = 23;
/// "Bonus Damage Resistance" (`+0x824`).
pub const BONUS_DAMAGE_RESISTANCE: u8 = 24;
/// "Icon ID" (`+0x5ea`).
pub const ICON_ID: u8 = 25;
/// "Amount of 2nd Resource Storage" (`+0x614`).
pub const STORAGE_2: u8 = 26;
/// "Amount of 3rd Resource Storage" (`+0x618`).
pub const STORAGE_3: u8 = 27;
/// "Fog Visibility" (`+0x605`).
pub const FOG_VISIBILITY: u8 = 28;
/// "Occlusion Mode" (`+0x650`).
pub const OCCLUSION_MODE: u8 = 29;
/// "Garrison Type" (`+0x9bc`).
pub const GARRISON_TYPE: u8 = 30;
/// "Unit Size Z" (`+0x5e0`).
pub const SIZE_Z: u8 = 32;
/// "Can Be Built On": the masters' setters write `+0x62b` with it, as for 45.
pub const CAN_BE_BUILT_ON: u8 = 33;
/// "Foundation Terrain" (`+0x988`).
pub const FOUNDATION_TERRAIN: u8 = 34;
/// "Hero Status" (`+0x8d9`).
pub const HERO_STATUS: u8 = 40;
/// "Frame Delay" (`+0x842`).
pub const FRAME_DELAY: u8 = 41;
/// "Train Location" (the selected train location's `+0x0`).
pub const TRAIN_LOCATION: u8 = 42;
/// "Train Button" (its `+0x4`).
pub const TRAIN_BUTTON: u8 = 43;
/// "Blast Attack Level" (`+0x830`).
pub const BLAST_ATTACK_LEVEL: u8 = 44;
/// "Blast Defense Level" (`+0x62b`).
pub const BLAST_DEFENSE_LEVEL: u8 = 45;
/// "Shown Attack" (`+0x856`).
pub const SHOWN_ATTACK: u8 = 46;
/// "Shown Range" (`+0x858`).
pub const SHOWN_RANGE: u8 = 47;
/// "Shown Melee Armor" (`+0x854`).
pub const SHOWN_MELEE_ARMOR: u8 = 48;
/// "Shown Pierce Armor" (`+0x8b0`).
pub const SHOWN_PIERCE_ARMOR: u8 = 49;
/// "Object Name ID" (`+0xc`).
pub const OBJECT_NAME_ID: u8 = 50;
/// "Short Description ID" (`+0x10`).
pub const SHORT_DESCRIPTION_ID: u8 = 51;
/// "Terrain Restriction ID" (`+0x606`).
pub const TERRAIN_RESTRICTION_ID: u8 = 53;
/// "Unit Trait" (`+0x654`).
pub const UNIT_TRAIT: u8 = 54;
/// "Trait Piece" (`+0x654`'s high half).
pub const TRAIT_PIECE: u8 = 56;
/// "Dead Unit ID" (`+0x5e4`).
pub const DEAD_UNIT_ID: u8 = 57;
/// "Hotkey ID" (the selected train location's `+0x8`).
pub const HOTKEY_ID: u8 = 58;
/// "Maximum Charge" (`+0x918`).
pub const MAXIMUM_CHARGE: u8 = 59;
/// "Recharge Rate" (`+0x91c`).
pub const RECHARGE_RATE: u8 = 60;
/// "Charge Event" (`+0x920`).
pub const CHARGE_EVENT: u8 = 61;
/// "Charge Type" (`+0x922`).
pub const CHARGE_TYPE: u8 = 62;
/// "Combat Ability" (`+0x840`).
pub const COMBAT_ABILITY: u8 = 63;
/// "Attack Dispersion" (`+0x860`).
pub const ATTACK_DISPERSION: u8 = 64;
/// "Secondary Projectile Unit" (`+0x8d4`).
pub const SECONDARY_PROJECTILE_UNIT: u8 = 65;
/// "Blood Unit" (`+0x5e6`).
pub const BLOOD_UNIT: u8 = 66;
/// "Projectile Hit Mode" (`+0x882`).
pub const PROJECTILE_HIT_MODE: u8 = 67;
/// "Projectile Vanish Mode" (`+0x883`).
pub const PROJECTILE_VANISH_MODE: u8 = 68;
/// "Projectile Arc" (`+0x888`).
pub const PROJECTILE_ARC: u8 = 69;
/// "Attack Graphic" (`+0x7f8`).
pub const ATTACK_GRAPHIC: u8 = 70;
/// "Standing Graphic" (`+0x470`).
pub const STANDING_GRAPHIC: u8 = 71;
/// "Standing Graphic 2" (`+0x478`).
pub const STANDING_GRAPHIC_2: u8 = 72;
/// "Dying Graphic" (`+0x480`).
pub const DYING_GRAPHIC: u8 = 73;
/// "Undead Graphic" (`+0x488`).
pub const UNDEAD_GRAPHIC: u8 = 74;
/// "Walking Graphic" (`+0x748`).
pub const WALKING_GRAPHIC: u8 = 75;
/// "Running Graphic" (`+0x750`).
pub const RUNNING_GRAPHIC: u8 = 76;
/// "Special Graphic" (`+0x8e8`).
pub const SPECIAL_GRAPHIC: u8 = 77;
/// "Obstruction Type" (`+0x651`).
pub const OBSTRUCTION_TYPE: u8 = 78;
/// "Blockage Class" (`+0x652`).
pub const BLOCKAGE_CLASS: u8 = 79;
/// "Selection Effect" (`+0x684`).
pub const SELECTION_EFFECT: u8 = 80;
/// "Special Ability" (`+0x8d8`).
pub const SPECIAL_ABILITY: u8 = 81;
/// "Idle Attack Graphic" (`+0x910`).
pub const IDLE_ATTACK_GRAPHIC: u8 = 82;
/// "Hero Glow Graphic" (`+0x908`).
pub const HERO_GLOW_GRAPHIC: u8 = 83;
/// "Garrison Graphic" (`+0x8f0`).
pub const GARRISON_GRAPHIC: u8 = 84;
/// "Construction Graphic", the first of a building's six sprites (85 to 90, `+0x950` to
/// `+0x978`).
pub const CONSTRUCTION_GRAPHIC: u8 = 85;
/// "Research Completed Graphic", the last of them.
pub const RESEARCH_COMPLETED_GRAPHIC: u8 = 90;
/// "Damage Graphic" (the selected damage sprite's sprite).
pub const DAMAGE_GRAPHIC: u8 = 91;
/// "Selection Sound", the first of the sounds 92 to 99.
pub const SELECTION_SOUND: u8 = 92;
/// "Damage Sound Event", the last of them.
pub const DAMAGE_SOUND_EVENT: u8 = 99;
/// "Resource Costs" (each paid cost entry, `+0x880`..`+0x890`).
pub const RESOURCE_COSTS: u8 = 100;
/// "Train Time" (the selected train location's `+0x2`).
pub const TRAIN_TIME: u8 = 101;
/// "Total Missiles" (`+0x8c0`).
pub const TOTAL_MISSILES: u8 = 102;
/// "Food Costs" (the cost entries of resource 0).
pub const FOOD_COSTS: u8 = 103;
/// "Wood Costs" (resource 1).
pub const WOOD_COSTS: u8 = 104;
/// "Gold Costs" (resource 3).
pub const GOLD_COSTS: u8 = 105;
/// "Stone Costs" (resource 2).
pub const STONE_COSTS: u8 = 106;
/// "Maximum Total Missiles" (`+0x8c4`).
pub const MAXIMUM_TOTAL_MISSILES: u8 = 107;
/// "Garrison Heal Rate" (`+0x9c0`).
pub const GARRISON_HEAL_RATE: u8 = 108;
/// "Regeneration Rate" (`+0x8b4`).
pub const REGENERATION_RATE: u8 = 109;
/// "Population" (the unpaid cost entries and the storage of resources 4, 11 and 19).
pub const POPULATION: u8 = 110;
/// "Minimum Conversion Time Modifier" (`+0x934`).
pub const MIN_CONVERSION_TIME_MODIFIER: u8 = 111;
/// "Maximum Conversion Time Modifier" (`+0x938`).
pub const MAX_CONVERSION_TIME_MODIFIER: u8 = 112;
/// "Conversion Chance Modifier" (`+0x93c`).
pub const CONVERSION_CHANCE_MODIFIER: u8 = 113;
/// "Formation Category" (`+0x8bc`).
pub const FORMATION_CATEGORY: u8 = 114;
/// "Area Damage" (`+0x834`).
pub const AREA_DAMAGE: u8 = 115;
/// "Damage Reflection" (`+0x864`).
pub const DAMAGE_REFLECTION: u8 = 118;
/// "Friendly Fire Damage" (`+0x868`).
pub const FRIENDLY_FIRE_DAMAGE: u8 = 119;
/// "Regeneration HP Percent" (`+0x8b4` from the hit points).
pub const REGENERATION_HP_PERCENT: u8 = 120;
/// "Ability Button Icon ID" (`+0x8da`), the first of the ability's interface values (121
/// to 124).
pub const ABILITY_BUTTON_ICON_ID: u8 = 121;
/// "Ability Hotkey Action" (`+0x8e4`), the last of them.
pub const ABILITY_HOTKEY_ACTION: u8 = 124;
/// "Charge Projectile Unit" (`+0x928`).
pub const CHARGE_PROJECTILE_UNIT: u8 = 125;
/// "Available Unit Flag" (`+0x5f0`).
pub const AVAILABLE_UNIT_FLAG: u8 = 126;
/// "Disabled Unit Flag" (`+0x5f1`).
pub const DISABLED_UNIT_FLAG: u8 = 127;
/// "Attack Priority" (`+0x92c`).
pub const ATTACK_PRIORITY: u8 = 128;
/// "Invulnerability Level" (`+0x930`).
pub const INVULNERABILITY_LEVEL: u8 = 129;
/// "Garrison Firepower" (`+0x870`).
pub const GARRISON_FIREPOWER: u8 = 130;
/// "Attack Graphic 2" (`+0x878`).
pub const ATTACK_GRAPHIC_2: u8 = 131;
/// "Command Sound", the first of the sounds 132 to 139.
pub const COMMAND_SOUND: u8 = 132;
/// "Transform Sound Event", the last of them.
pub const TRANSFORM_SOUND_EVENT: u8 = 139;
/// "Run Pattern" (`+0x7b9`).
pub const RUN_PATTERN: u8 = 140;
/// "Interface Kind" (`+0x62f`).
pub const INTERFACE_KIND: u8 = 141;
/// "Combat Level" (`+0x62c`).
pub const COMBAT_LEVEL: u8 = 142;
/// "Interaction Mode" (`+0x62d`).
pub const INTERACTION_MODE: u8 = 143;
/// "Minimap Mode" (`+0x62e`).
pub const MINIMAP_MODE: u8 = 144;
/// "Trailing Unit" (`+0x75e`).
pub const TRAILING_UNIT: u8 = 145;
/// "Trail Mode" (`+0x760`).
pub const TRAIL_MODE: u8 = 146;
/// "Trail Density" (`+0x764`).
pub const TRAIL_DENSITY: u8 = 147;
/// "Projectile Graphic Displacement X" (`+0x844`; Y and Z follow).
pub const DISPLACEMENT_X: u8 = 148;
/// "Projectile Graphic Displacement Y" (`+0x848`).
pub const DISPLACEMENT_Y: u8 = 149;
/// "Projectile Graphic Displacement Z" (`+0x84c`).
pub const DISPLACEMENT_Z: u8 = 150;
/// "Projectile Spawning Area Width" (`+0x8c8`; the length and the randomness follow).
pub const SPAWNING_AREA_WIDTH: u8 = 151;
/// "Projectile Spawning Area Length" (`+0x8cc`).
pub const SPAWNING_AREA_LENGTH: u8 = 152;
/// "Projectile Spawning Area Randomness" (`+0x8d0`).
pub const SPAWNING_AREA_RANDOMNESS: u8 = 153;
/// "Damage Graphics Entry Mod" (`+0x731`).
pub const DAMAGE_GRAPHICS_ENTRY_MOD: u8 = 154;
/// "Damage Graphics Total Num" (`+0x638`'s count).
pub const DAMAGE_GRAPHICS_TOTAL_NUM: u8 = 155;
/// "Damage Graphic Percent" (the selected damage sprite's `+0x8`).
pub const DAMAGE_GRAPHIC_PERCENT: u8 = 156;
/// "Damage Graphic Apply Mode" (its `+0x9`).
pub const DAMAGE_GRAPHIC_APPLY_MODE: u8 = 157;
/// "Train Locations Entry Mod" (`+0x940`).
pub const TRAIN_LOCATIONS_ENTRY_MOD: u8 = 158;
/// "Train Locations Total Num" (`+0x898`'s count).
pub const TRAIN_LOCATIONS_TOTAL_NUM: u8 = 159;

/// The lane of an attribute that sets one of three consecutive attributes' values: `first` and
/// `second` name the first two lanes, any other attribute of the range is the third.
pub(crate) const fn lane_mut(
    lanes: &mut [f32; 3],
    attribute: u8,
    (first, second): (u8, u8),
) -> &mut f32 {
    let [one, two, three] = lanes;
    if attribute == first {
        one
    } else if attribute == second {
        two
    } else {
        three
    }
}
