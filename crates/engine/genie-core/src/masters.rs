//! What the engine reads of a player's masters: the tasks a unit can do and the masters a unit
//! changes into to do them, and how an effect changes a master: the attribute setters (slots
//! 0x18, 0x20 and 0x28 of every master type, [`Master::set_attribute`]), the master copy of an
//! upgrade (slot 8, [`Master::copy_from`]) and the values the slice derives from a master
//! ([`Master::follow`]). The values are game data: the player's masters in the save, with the
//! `.dat`'s values for what the save does not carry.

use crate::GenieError;
use crate::movement::MoveStats;
use crate::tasks::{Actor, Diplomacy, TaskTarget, find_task};

/// One task of a master's task list (`RGE_Task`), with the memory offsets the task finder reads
/// (the save's names; the `.dat`'s in brackets where they differ).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Task {
    /// `+0xa`.
    pub id: i16,
    /// `+0xe`.
    pub action_type: i16,
    /// `+0x10`: the target class the task is for, -1 for any.
    pub object_group: i16,
    /// `+0x12`: the target master, -1 for any.
    pub object_id: i16,
    /// `+0x14`, `combat_level` (`enable_targeting`): the task is for a target's combat level.
    pub combat_level: u8,
    /// `+0x15`, `combat_level_flag`: a generation task (151, [`crate::abilities`]) of a combat
    /// unit with bit 1 or of a building with bit 2 works on its own; a removal reward task (161)
    /// not 0 pays the master's cost of its resource times the rate.
    pub combat_level_flag: u8,
    /// `+0x16`: the terrain the task is for, -1 for none.
    pub terrain: i16,
    /// `+0x18`, `owner_type` (`target_diplomacy`): whose targets.
    pub owner_type: u8,
    /// `+0x19`, `holding_attr` (`carry_check`): 1 wants a target holding the task's resource.
    pub holding_attr: u8,
    /// `+0x20` to `+0x26`, `attribute_type` to `attribute_type4` (`resource_in`,
    /// `resource_multiplier`: the attribute that scales the gathering, `resource_out`,
    /// `unused_resource`).
    pub attribute_type: i16,
    /// `+0x22`, `attribute_type2` (`resource_multiplier`): the attribute that scales the gathering.
    pub attribute_type2: i16,
    /// `+0x24`, `attribute_type3` (`resource_out`).
    pub attribute_type3: i16,
    /// `+0x26`, `attribute_type4` (`unused_resource`).
    pub attribute_type4: i16,
    /// `+0x1c`, `search_wait_time`: a Fly task's rest between flights.
    pub wait_time: f32,
    /// `+0x28`, the save's `work_val1`; `+0x2c`, `work_val2`.
    pub work_val1: f32,
    /// `+0x2c`, the save's `work_val2`.
    pub work_val2: f32,
    /// `+0x30`: how near the target the work is done.
    pub work_range: f32,
    /// `+0x34`, `work_flag` (the `.dat`'s `gather_type`): a Make adds to its player's attribute,
    /// not to its object's holding ([`crate::make`]).
    pub work_flag: i16,
    /// `+0x38`, `enablement_resource_id` (`enabled`): the resource that enables the task, -1 none.
    pub enablement: i16,
    /// `+0x40`..`+0x58`: the moving, proceeding, working and carrying sprites (graphic ids, -1 for
    /// none).
    pub sprites: [i16; 4],
    /// `+0x60` and `+0x78`: the event ids (`mEventId`) of the gathering and the deposit sound,
    /// as the task's load (`FUN_141062470`) leaves them; a terrain look task (159) names its
    /// dying and its undead sprite with them ([`crate::abilities`]).
    pub sound_events: [i32; 2],
}

/// A task for any target, with no resource, terrain or requirement.
impl Default for Task {
    fn default() -> Self {
        Self {
            id: 0,
            action_type: 0,
            object_group: -1,
            object_id: -1,
            combat_level: 0,
            combat_level_flag: 0,
            terrain: -1,
            owner_type: 0,
            holding_attr: 0,
            attribute_type: -1,
            attribute_type2: -1,
            attribute_type3: -1,
            attribute_type4: -1,
            wait_time: 0.0,
            work_val1: 0.0,
            work_val2: 0.0,
            work_range: 0.0,
            work_flag: 0,
            enablement: -1,
            sprites: [-1; 4],
            sound_events: [0; 2],
        }
    }
}

/// The task types the action list's factory makes an action of: the TRIBE factory's
/// (`FUN_140aca4b0`) and the RGE factory's it falls back on (`FUN_140a97e00`). A default task of
/// any other type starts nothing (wild animals' Explore, type 6).
pub const ACTION_FACTORY_TYPES: [i16; 28] = [
    3, 0xc, 0xe, 0x65, 0x68, 0x69, 0x6a, 0x6b, 0x6c, 0x6e, 0x6f, 0x78, 0x79, 0x7a, 0x7b, 0x7d,
    0x83, 0x84, 0x87, 0x88, 0x95, 0x9a, 4, 5, 7, 10, 0xd, 0x15,
];

impl Master {
    /// Its default task (`FUN_14109e0c0`: the task list's entry at the default task's index,
    /// `+0x788`), if it has one.
    #[must_use]
    pub fn default_task(&self) -> Option<&Task> {
        self.tasks.get(usize::try_from(self.default_task?).ok()?)
    }

    /// What a new object of this master starts once its action list is empty in state 2
    /// (`RGE_Action_Object::update`, slot 0x7f0): a Make ([`crate::make`]: true), or the
    /// default task's action the slice does not model; nothing for a type the factory makes no
    /// action of.
    #[must_use]
    pub fn default_start(&self) -> (bool, Option<GenieError>) {
        match self.default_task() {
            Some(t) if t.action_type == crate::make::MAKE => (true, None),
            Some(t) if ACTION_FACTORY_TYPES.contains(&t.action_type) => (
                false,
                Some(GenieError::unsupported(
                    "a new object's default task other than a Make",
                )),
            ),
            _ => (false, None),
        }
    }
}

/// What a building's completion (`FUN_1411dbb70` with 2) reads of its master, the `.dat`'s type
/// 80 values.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BuildingMaster {
    /// `+0x980`, `adjacent_mode`: walls and gates join their neighbours (`FUN_1411ce890`).
    pub adjacent_mode: u8,
    /// `+0x984`, `disappears_when_built`.
    pub disappears_when_built: bool,
    /// `+0x986`, `stack_unit_id`: the building becomes another (−1 for none).
    pub stack_unit: i16,
    /// `+0x988`, `foundation_terrain_id` (the save's `on_build_make_tile`; −1 for none).
    pub foundation_terrain: i16,
    /// `+0x98c`, `tech_id`: the technology its completion researches (−1 for none).
    pub tech: i16,
    /// `+0x9bc`, `garrison_type`: the kinds of units it takes in (bit 0 villagers, `FUN_14120eb10`).
    pub garrison_type: u8,
    /// `+0x9c0`, `garrison_heal_rate`: the hit points a second it gives the units inside.
    pub garrison_heal_rate: f32,
    /// `+0x9c4`, `garrison_repair_rate`: its repair a second per villager inside.
    pub garrison_repair_rate: f32,
    /// `+0x990`, the masters of its annexes (−1 for none; their places beside it at `+0x998`).
    pub annexes: [i16; 4],
    /// `+0x998` (x) and `+0x9a8` (y), each annex slot's place off the building's (the `.dat`'s
    /// building annexes' `misplacement`), which the building constructor (`FUN_1411de5f0`) makes
    /// its annexes at.
    pub annex_offsets: [[f32; 2]; 4],
    /// `+0x9b8`, `head_unit`: the building it is an annex of (−1 for none).
    pub head_unit: i16,
    /// `+0x9ba`, `transform_unit`: the master it transforms into (−1 for none); a gate with one
    /// is in its player's gate list (`+0x1360`, `FUN_141616200`).
    pub transform_unit: i16,
    /// `+0x960`, the `.dat`'s `destruction_graphic_id` (the save's third building sprite): the
    /// sprite a building that dies before it is built shows (`RGE_Static_Object::update`), −1
    /// for none.
    pub destruction_sprite: i16,
}

/// The master values the task finder and the work command read.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Master {
    /// The master's id.
    pub id: i16,
    /// The master's type (`+0x8`): 70 for a combat unit, 80 (`'P'`) for a building.
    pub kind: u8,
    /// `+0x158`, the `.dat`'s `copy_id`: the master a garrison order asks a building about
    /// (`FUN_141621220`).
    pub copy_id: i16,
    /// `+0x260`, the `.dat`'s `base_id` (the master load `FUN_1411319f0` turns 768 into 420 and
    /// 770 into 691): the master whose availability says whether the player can make this one
    /// (`FUN_141663f80`, [`crate::turn::Turns::can_make`]); an upgrade's copy keeps it.
    pub base_id: i16,
    /// `+0x410`.
    pub class: i16,
    /// `+0x7b8`, the `.dat`'s `TaskSwapGroup` (the save does not carry it); 0 for none.
    pub task_by_group: u8,
    /// `radius_x`, `radius_y` (`+0x5d8`, `+0x5dc`).
    pub radius: [f32; 2],
    /// `radius_z` (`+0x5e0`) is above 0: its objects count as tall on their tiles.
    pub tall: bool,
    /// `radius_z` (`+0x5e0`): how high a missile may pass over its objects and hit them
    /// (`FUN_14113f740`).
    pub height: f32,
    /// `line_of_sight` (`+0x5d0`).
    pub line_of_sight: f32,
    /// `+0x606`, its terrain restriction (the save's `terrain_restriction_mask_id`; −1 for none),
    /// which a placement test reads.
    pub terrain_restriction: i16,
    /// `obstruction_type` (`+0x651`) and `can_be_built_on` (`+0x5e9`).
    pub obstruction_type: u8,
    /// `can_be_built_on` (`+0x5e9`): a new building deletes an object of it off its footprint.
    pub buildable_on: u8,
    /// `obstruction_class` (`+0x652`).
    pub obstruction_class: u8,
    /// `object_flags` (`+0x654`): bit 0 takes garrisons, bit 4 marks a hunter master.
    pub object_flags: u32,
    /// The `.dat`'s `combat_level` (`+0x62c`).
    pub combat_level: u8,
    /// Its task list.
    pub tasks: Vec<Task>,
    /// `+0x790`, `default_task`: the id of the task its object's empty action list starts in
    /// state 2 (`RGE_Action_Object::update`, slot 0x7f0); None for the game's −1.
    pub default_task: Option<i16>,
    /// The build cost `(resource, amount, paid)`.
    pub cost: Vec<(i16, i16, bool)>,
    /// `work_rate` (`+0x798`): what a builder of this master adds to the work per second.
    pub work_rate: f32,
    /// `search_radius` (`+0x794`, the object's slot 0x2c8).
    pub search_radius: f32,
    /// The `.dat`'s `drop_sites` (`+0x7a0`..`+0x7a8`, `FUN_14109d4f0`): the buildings a unit of
    /// this master takes its load to.
    pub drop_sites: Vec<i16>,
    /// The hit points (`+0x530`) and the build time: the first train location's train time
    /// (`+0x898`'s first entry), 0 without one.
    pub hit_points: i16,
    /// The build time: the first train location's train time, 0 without one.
    pub build_time: i16,
    /// The train locations' ids (`+0x898`, each entry's first short): the units that build a
    /// building, the buildings that train a unit (`FUN_141263610`).
    pub trained_at: Vec<i16>,
    /// The combat master's `weapon_range` (`+0x828`, object slot 0x230) and
    /// `minimum_weapon_range` (`+0x850`, slot 0x240).
    pub weapon_range: f32,
    /// `minimum_weapon_range` (`+0x850`, slot 0x240).
    pub min_range: f32,
    /// `charge_type` (`+0x922`): 6 and 7 add `charge_event` (`+0x920`) to the range against some
    /// targets (slot 0x2f8).
    pub charge_type: i16,
    /// `+0x61c`, `attribute_max_amount`: how much an object of this master carries.
    pub capacity: i16,
    /// `+0x8bc`, `mFormationCategory`: 0 (villagers) keeps a unit out of the groups an order's
    /// splitter forms (`FUN_140fd76d0`).
    pub formation_category: u8,
    /// `+0x8d9`, `hero_flag`: a combat master with bit 0 or 4 set and not both 4 and 7 joins a
    /// group's fifth sub-formation (`FUN_141011300`).
    pub hero_flag: u8,
    /// `+0x8b8`, `mSpacingModifier`: with the radius on x the unit's radius in a group's
    /// formation (`FUN_141025920`). The player's masters copy it from the data file's.
    pub spacing_modifier: f32,
    /// `+0x688`, `+0x68c`: the `.dat`'s `outline_size` (x, y), an obstruction-type-11 object's
    /// shape radii (`FUN_141054fe0`).
    pub outline: [f32; 2],
    /// `+0x5fc`, `clearance_size`: a completed building's foundation terrain covers its position
    /// plus and minus this.
    pub clearance: [f32; 2],
    /// `+0x820`, the `.dat`'s `defense_terrain_bonus` (a terrain restriction): a completed
    /// building keeps to restore a tile's terrain whose value in it is neither 0 nor 1.
    pub terrain_bonus: i16,
    /// What a building's completion reads (type 80), None for other masters.
    pub building: Option<BuildingMaster>,
    /// The resource storage (`attribute_type_held`, `attribute_amount_held`, `attribute_flag`:
    /// `+0x60a`, `+0x610`, `+0x628`): what an object of this master adds to its player's
    /// attributes, by its flag.
    pub storage: Vec<(i16, f32, u8)>,
    /// The master has damage sprites (`+0x638`..`+0x640`): a master change checks them
    /// (`FUN_1411859a0`).
    pub damage_sprites: bool,
    /// The `.dat`'s `fly_mode` (`+0x608`): 1 flies above the ground (a bird's Fly lifts it).
    pub fly_mode: u8,
    /// What a hit on an object of this master reads (`FUN_141082530`): its armor classes (class,
    /// amount; `+0x808`, count `+0x802`), its base armor for a class it does not list (`+0x800`),
    /// `bonus_damage_resistance` (`+0x824`), `combat_ability` (`+0x840`), `damage_reflection`
    /// (`+0x864`), `invulnerability_level` (`+0x930`: a hit point floor, `FUN_141202a80`) and
    /// `special_attack_flag` (`+0x8d8`), `friendly_fire_damage` (`+0x868`), its special graphic
    /// (`+0x8e8`, the `.dat`'s `special_graphic`) and `area_effect_object_level` (`+0x62b`, the
    /// blasts that reach it); and what an attacker of it reads: `area_effect_range` (`+0x82c`),
    /// `area_effect_level` (`+0x830`), `area_damage` (`+0x834`), `max_charge` (`+0x918`) and
    /// `charge_target` (`+0x924`).
    pub base_armor: i16,
    /// Its armor classes (class, amount; `+0x808`, count `+0x802`).
    pub armors: Vec<(i16, i16)>,
    /// `bonus_damage_resistance` (`+0x824`).
    pub resistance: f32,
    /// `combat_ability` (`+0x840`).
    pub combat_ability: u8,
    /// `damage_reflection` (`+0x864`).
    pub reflection: f32,
    /// `invulnerability_level` (`+0x930`): a hit point floor.
    pub invulnerability: f32,
    /// `attack_priority` (`+0x92c`, `FUN_141202220`): 3 attacks only the classes of
    /// `FUN_140fddf10`.
    pub attack_priority: u8,
    /// `special_attack_flag` (`+0x8d8`).
    pub special_attack: u8,
    /// `friendly_fire_damage` (`+0x868`).
    pub friendly_fire: f32,
    /// It has a special graphic (`+0x8e8`).
    pub special_graphic: bool,
    /// The special graphic's sprite (the `.dat`'s `special_graphic`), −1 for none: a herder
    /// leading livestock shows it.
    pub special_sprite: i16,
    /// `area_effect_object_level` (`+0x62b`): the blasts that reach it.
    pub blast_defense: u8,
    /// `area_effect_range` (`+0x82c`): its blast's radius.
    pub area_effect: f32,
    /// `area_effect_level` (`+0x830`).
    pub blast_level: u8,
    /// `area_damage` (`+0x834`).
    pub area_damage: f32,
    /// `max_charge` (`+0x918`).
    pub max_charge: f32,
    /// `charge_target` (`+0x924`).
    pub charge_target: i16,
    /// `+0x5d4`, `obj_max` (the `.dat`'s `garrison_capacity`): a building that garrisons takes a
    /// gather point (`FUN_140e7e9a0`).
    pub garrison: u8,
    /// `+0x680`, the `.dat`'s `recyclable`: its objects take ids from the negative range
    /// (`FUN_1415301b0`).
    pub recyclable: bool,
    /// `+0x681`, `enable_auto_gather`: an object of it blocking a spawn point stays
    /// (`FUN_141194170`).
    pub auto_gather: bool,
    /// `+0x683`, `resource_gather_group`: the resource kind a gatherer's search
    /// (`ClosestResourceObject`) takes it for (0 wood, 1 food, 3 stone, 4 gold, 2 the rest).
    pub gather_group: u8,
    /// `+0x604`, `hill_mode`: how a placement test checks the elevation.
    pub hill_mode: u8,
    /// `+0x605`, `fog_visibility`: an object of it the player only explored stays a target
    /// (the Attack's update).
    pub fog_visibility: u8,
    /// `+0x5f0`, `available`: the player may make it (an effect's enable command, kind 2,
    /// changes it).
    pub available: bool,
    /// `+0x5f1`, `disabled`: an enable command leaves `available` as it is.
    pub disabled: bool,
    /// `+0x682`, `create_doppelganger_on_death`: 2 leaves ghosts of an object of it for the
    /// players who explored its tile but do not see it, at its death and once they lose sight
    /// of it ([`crate::ghost`]).
    pub ghost_on_death: u8,
    /// The placement terrains (`+0x5f2`..`+0x5f8`, the `.dat`'s `placement_side_terrain` and
    /// `placement_terrain`, −1 for none): the two side terrains, then the two terrains
    /// ([`crate::spawn::placement_terrains`]).
    pub placement: [i16; 4],
    /// The death sprite (`+0x480`, −1 for none) and the death spawns (`+0x5e4`, `+0x5e6`, −1 for
    /// none), and what a dead object's steps read of the master
    /// ([`crate::lifecycle::MasterData`]), for a new object's [`crate::lifecycle::Lifecycle`].
    pub death_sprite: i16,
    /// The death spawns (`+0x5e4`, `+0x5e6`, −1 for none).
    pub death_spawns: [i16; 2],
    /// What a dead object's steps read of the master.
    pub life: Option<crate::lifecycle::MasterData>,
    /// What a combat master's attack reads: its standing sprite (`+0x470`), its attack sprite
    /// (`+0x7f8`, `fight_sprite_id`), a second attack sprite
    /// (`+0x878`, the `.dat`'s `attack_graphic2`), `missile_id`, `fire_missile_at_frame`,
    /// `interrupt_frame`, `base_hit_chance`, `speed_of_attack` (the reload time) and its weapons
    /// (class, amount); and the sprite it waits with between swings (`+0x910`, the `.dat`'s
    /// `idle_attack_graphic`, −1 for none).
    pub standing_sprite: i16,
    /// Its second standing sprite (`+0x478`, the `.dat`'s `standing_graphic[1]`), whose idle
    /// animation the object swaps in (`0x141071880`); None for none.
    pub standing_sprite_2: Option<i16>,
    /// The sprite it waits with between swings (`+0x910`, −1 for none).
    pub idle_attack_sprite: i16,
    /// Its walking sprite (`+0x748`, `move_sprite_id`; the unit's slot 0x7c8) and its running
    /// one (`+0x750`, `run_sprite_id`; slot 0x7d0), −1 for none.
    pub move_sprite: i16,
    /// Its running sprite (`+0x750`), −1 for none.
    pub run_sprite: i16,
    /// Its attack sprite (`+0x7f8`, `fight_sprite_id`).
    pub attack_sprite: i16,
    /// A second attack sprite (`+0x878`).
    pub attack_sprite2: i16,
    /// `missile_id`, −1 for none.
    pub missile: i16,
    /// `fire_missile_at_frame`.
    pub fire_frame: i16,
    /// `interrupt_frame`.
    pub interrupt_frame: i16,
    /// `base_hit_chance`.
    pub hit_chance: i16,
    /// `speed_of_attack`: the reload time.
    pub reload: f32,
    /// Its weapons (class, amount).
    pub weapons: Vec<(i16, i16)>,
    /// Where its missile starts from its place, turned by its facing (`+0x844`..`+0x84c`,
    /// `weapon_offset`), and how far a missed missile strays per tile of the way
    /// (`missed_missile_spread`, `+0x860`).
    pub weapon_offset: [f32; 3],
    /// How far a missed missile strays per tile of the way (`missed_missile_spread`, `+0x860`).
    pub missed_spread: f32,
    /// `garrison_firepower` (`+0x870`): what a unit adds to the volleys of a building it is
    /// inside; one at most 0 adds its magnitude.
    pub garrison_firepower: f32,
    /// `volley_fire_amount` (`+0x8c0`, the `.dat`'s total projectiles): a new object's missiles
    /// per volley (`+0xa00`).
    pub volley_fire: f32,
    /// `max_attacks_in_volley` (`+0x8c4`, the `.dat`'s max total projectiles).
    pub max_volley: u8,
    /// `volley_x_spread`, `volley_y_spread` and `volley_start_spread_adjustment` (`+0x8c8`..
    /// `+0x8d0`, the `.dat`'s projectile spawning area): how far a volley's missiles scatter.
    pub volley_spread: [f32; 3],
    /// `volley_missile_id` (`+0x8d4`, the `.dat`'s secondary projectile), None for −1.
    pub volley_missile: Option<i16>,
    /// `mChargeProjectileUnit` (`+0x928`), None for −1.
    pub charge_projectile: Option<i16>,
    /// A missile master's flight (type 60, [`crate::missile::MissileStats`]).
    pub missile_stats: Option<crate::missile::MissileStats>,
    /// What a unit of this master moves with, its speed with the master's walking sprite (the
    /// run sprite `+0x750` if it has one, else the move sprite `+0x748`); None for a master that
    /// does not move.
    pub moving: Option<MoveStats>,
    /// `+0x740`, `mMaxSpeed` (the `.dat`'s `speed`): the top speed before a sprite's factor, from
    /// which [`Master::moving`]'s speeds and [`crate::missile::MissileStats::speed`] follow.
    pub max_speed: f32,
    /// `+0x758`, `turn_speed` (the `.dat`'s `rotation_speed`), which [`MoveStats::turn_time`]
    /// follows.
    pub turn_speed: f32,
    /// `+0x488`, the undead sprite (`undeadSpriteId`, −1 for none).
    pub undead_sprite: i16,
    /// `+0x650`, `occlusion_flag` (the `.dat`'s `occlusion_mode`).
    pub occlusion: u8,
    /// `+0x684`, the `.dat`'s `selection_effect`.
    pub selection_effect: u8,
    /// `+0x62f`, `unit_level` (the `.dat`'s `interface_kind`).
    pub unit_level: u8,
    /// `+0x62d`, `select_level` (the `.dat`'s `interaction_mode`).
    pub select_level: u8,
    /// `+0x62e`, `map_draw_level` (the `.dat`'s `minimap_mode`).
    pub map_draw_level: u8,
    /// The damage sprites (`+0x638`, 16 bytes each; the save's `damage_sprite`), which
    /// [`Master::damage_sprites`] follows.
    pub damage_graphics: Vec<DamageGraphic>,
    /// `+0x731`, `mDamageSpriteEntryMod`: the damage sprite the attributes 91, 156 and 157 change.
    pub damage_graphic_entry: u8,
    /// A moving master's trail (`+0x75e`..`+0x764`).
    pub trail: Trail,
    /// `+0x7b9`, `run_pattern`.
    pub run_pattern: u8,
    /// `+0x856`, `+0x858`, `+0x854` and `+0x8b0`: the attack, range, melee and pierce armor a
    /// combat master shows (`orig_weapon`, `orig_weapon_range`, `orig_armor`,
    /// `orig_pierce_armor`).
    pub shown: Shown,
    /// The train locations (`+0x898`, 12 bytes each; the save's `train_location`), which
    /// [`Master::trained_at`] and [`Master::build_time`] follow.
    pub train_locations: Vec<TrainLocation>,
    /// `+0x940`, `mTrainLocationEntryMod`: the train location the attributes 42, 43, 58 and 101
    /// change.
    pub train_location_entry: i16,
    /// `+0x8b4`, `regeneration_rate`.
    pub regeneration: f32,
    /// `+0x91c`, `mRechargeRate`.
    pub recharge_rate: f32,
    /// `+0x920`, `mChargeEvent`.
    pub charge_event: i16,
    /// `+0x934`, `+0x938`, `+0x93c`: `mMinConversionTimeAdj`, `mMaxConversionTimeAdj`,
    /// `mConversionChanceAdj`.
    pub conversion: [f32; 3],
    /// `+0x908`, the hero glow sprite (−1 for none).
    pub hero_glow_sprite: i16,
    /// `+0x8f0`, the garrison sprite (−1 for none).
    pub garrison_sprite: i16,
    /// A building master's `+0x982`, `building_facet` (the attribute list's "Building Icon
    /// Override").
    pub building_facet: i16,
}

/// One damage sprite of a master (`+0x638`'s entries; the save's `damage_sprite`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DamageGraphic {
    /// `+0x0`, the sprite (`tempId`, −1 for none).
    pub sprite: i16,
    /// `+0x8`, `damage_percent`.
    pub percent: u8,
    /// `+0x9`, the `flag` (the `.dat`'s `apply_mode`).
    pub apply_mode: u8,
}

/// A new damage sprite entry: no sprite, zero bytes (the game zeroes its 16 bytes).
impl Default for DamageGraphic {
    fn default() -> Self {
        Self {
            sprite: -1,
            percent: 0,
            apply_mode: 0,
        }
    }
}

/// One train location of a combat master (`+0x898`'s 12-byte entries; the save's
/// `train_location`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TrainLocation {
    /// `+0x0`, `trainLocationID`: the master that makes it.
    pub location: i16,
    /// `+0x2`, `trainTime`.
    pub time: i16,
    /// `+0x4`, `buttonLocationID`.
    pub button: u8,
    /// `+0x8`, `hotkeyID`.
    pub hotkey: i32,
}

/// A moving master's trail (`obj_trail_id`, `obj_trail_options`, `obj_trail_spacing`; the
/// `.dat`'s tracking unit, its mode and density).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Trail {
    /// `+0x75e`: the master it leaves behind.
    pub unit: i16,
    /// `+0x760`.
    pub mode: u8,
    /// `+0x764`.
    pub density: f32,
}

/// What a combat master shows of its values (`+0x856`, `+0x858`, `+0x854`, `+0x8b0`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Shown {
    /// `+0x856`, `orig_weapon`.
    pub attack: i16,
    /// `+0x858`, `orig_weapon_range`.
    pub range: f32,
    /// `+0x854`, `orig_armor`.
    pub melee_armor: i16,
    /// `+0x8b0`, `orig_pierce_armor`.
    pub pierce_armor: i16,
}

/// A player's masters by id.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Masters {
    by_id: Vec<Option<Master>>,
}

impl Masters {
    /// The masters, by their ids.
    pub fn new(masters: impl IntoIterator<Item = Master>) -> Self {
        let mut by_id: Vec<Option<Master>> = Vec::new();
        for m in masters {
            let i = m.id as usize;
            if by_id.len() <= i {
                by_id.resize(i + 1, None);
            }
            by_id[i] = Some(m);
        }
        Self { by_id }
    }

    /// The master `id`, if the player has it.
    #[must_use]
    pub fn get(&self, id: i16) -> Option<&Master> {
        self.by_id.get(usize::try_from(id).ok()?)?.as_ref()
    }

    /// Every master, by id.
    pub fn iter(&self) -> impl Iterator<Item = &Master> {
        self.by_id.iter().flatten()
    }

    /// Every master, by id, to change (a technology's effect).
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Master> {
        self.by_id.iter_mut().flatten()
    }

    /// The master `id` to change, if the player has it.
    pub fn get_mut(&mut self, id: i16) -> Option<&mut Master> {
        self.by_id.get_mut(usize::try_from(id).ok()?)?.as_mut()
    }

    /// The work command's choice of master and task (`FUN_14106ff70` up to the task's
    /// requirement): the unit's own master's task for the target ([`find_task`]) unless it has
    /// none or only an attack (7); then, with a task swap group, the first master by id of the
    /// unit's class, type and group with a task of another type (its master changes, slot 0x90),
    /// except that a gather task (106) for a farm, a pasture or the other objects the work command
    /// lists, with any filter, prefers the first such master with a task for the target's class
    /// and a work value, else the first with a task for any target. `Ok(None)`: no task.
    pub fn work_master<'m>(
        &'m self,
        actor: &Actor<'_>,
        target: &TaskTarget<'_>,
        general: &TaskTarget<'_>,
        filter: i16,
        dip: &impl Diplomacy,
    ) -> Result<Option<(i16, &'m Task)>, GenieError> {
        let own = actor.master;
        let own_list = self.get(own.id).ok_or(GenieError::unsupported(
            "a unit of a master the player does not have",
        ))?;
        let own_task = find_task(
            own_list,
            actor,
            Some(target),
            Some(general),
            None,
            filter,
            dip,
        )?;
        if own_task.is_none_or(|t| t.action_type == 7) && own.task_by_group != 0 {
            let (mut by_class, mut generic) = (None, None);
            for candidate in self.by_id.iter().flatten() {
                if candidate.class != own.class
                    || candidate.kind != own.kind
                    || candidate.task_by_group != own.task_by_group
                {
                    continue;
                }
                let found = find_task(
                    candidate,
                    actor,
                    Some(target),
                    Some(general),
                    None,
                    filter,
                    dip,
                )?;
                let Some(task) = found else { continue };
                if own_task.is_some_and(|t| t.action_type == task.action_type) {
                    continue;
                }
                let listed = matches!(
                    target.master.id,
                    50 | 357 | 1187 | 1188 | 1893 | 1894 | 1897 | 1898
                );
                if task.action_type != 0x6a
                    || (filter != -1 && filter != 0x6a)
                    || !listed
                    || task.object_id == target.master.id
                {
                    return Ok(Some((candidate.id, task)));
                }
                if by_class.is_none()
                    && task.object_group == target.master.class
                    && 0.0 < task.work_val1
                {
                    by_class = Some((candidate.id, task));
                } else if generic.is_none() && task.object_group == -1 && task.object_id == -1 {
                    generic = Some((candidate.id, task));
                }
            }
            if let Some(pick) = by_class.or(generic) {
                return Ok(Some(pick));
            }
        }
        Ok(own_task.map(|t| (own.id, t)))
    }
}

/// The attributes an effect changes on a master and on its objects (the game's attribute list:
/// string 12200 plus the number names each).
pub mod attribute {
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
    /// "Projectile Graphic Displacement Z" (`+0x84c`).
    pub const DISPLACEMENT_Z: u8 = 150;
    /// "Projectile Spawning Area Width" (`+0x8c8`; the length and the randomness follow).
    pub const SPAWNING_AREA_WIDTH: u8 = 151;
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
}

/// The master types (`+0x8`) whose attribute setters (slots 0x18, 0x20, 0x28) differ.
pub mod master_type {
    /// `RGE_Master_Static_Object`.
    pub const STATIC: u8 = 10;
    /// `TRIBE_Master_Tree_Object`, with the static object's setters.
    pub const TREE: u8 = 15;
    /// `RGE_Master_Animated_Object`.
    pub const ANIMATED: u8 = 20;
    /// `RGE_Master_Doppleganger_Object`, with the animated object's setters.
    pub const DOPPELGANGER: u8 = 25;
    /// `RGE_Master_Moving_Object`.
    pub const MOVING: u8 = 30;
    /// `RGE_Master_Action_Object`.
    pub const ACTION: u8 = 40;
    /// `RGE_Master_Combat_Object`.
    pub const RGE_COMBAT: u8 = 50;
    /// `RGE_Master_Missile_Object` (`'<'`).
    pub const MISSILE: u8 = 60;
    /// `TRIBE_Master_Combat_Object` (`'F'`).
    pub const COMBAT: u8 = 70;
    /// `TRIBE_Master_Building_Object` (`'P'`).
    pub const BUILDING: u8 = 80;
}

/// 32767.0 (`DAT_142eb69cc`): a capped add or multiply holds hit points and carry capacity
/// below it.
const SHORT_CAP: f32 = crate::image::masters::SHORT_CAP;
/// 0.5 (`DAT_142e7f384`): a multiply's rounding.
const ROUND: f32 = crate::image::masters::ROUND;
/// 4.5 (`DAT_142f52490`): the most speed a capped add or multiply leaves .
const SPEED_CAP: f32 = crate::image::masters::SPEED_CAP;
/// 20.0 (`DAT_142e7f38c`): a missile master's .
const MISSILE_SPEED_CAP: f32 = crate::image::masters::MISSILE_SPEED_CAP;
/// 1.0 (`DAT_14330eb8c`): a handicap above it raises an attack class at 0 .
const HANDICAP_FLOOR: f32 = crate::image::masters::HANDICAP_FLOOR;
/// 100.0 (`DAT_14330ebcc`): a multiply's armor or attack amount is a percentage (an image role to
/// add: `masters::PERCENT`).
const PERCENT: f32 = crate::image::masters::PERCENT;
/// The attack classes a handicap leaves alone (`0x280000018`'s bits up to class 0x21): 3, 4, 31
/// and 33.
const UNHANDICAPPED: u64 = 0x2_8000_0018;
/// The most damage sprites (`FUN_1410e87c0`, 0xff) and train locations (`FUN_141249220`,
/// 0x7fff) a resize leaves; an insert past one less is refused (`FUN_1410e8680`,
/// `FUN_141249040`).
const MOST_DAMAGE_GRAPHICS: u64 = 0xff;
const MOST_TRAIN_LOCATIONS: u64 = 0x7fff;
/// The obstruction types `FUN_141125ae0` keeps (its 14-byte table); any other becomes 0.
const VALID_OBSTRUCTIONS: [bool; 14] = [
    true, true, true, true, false, true, false, false, false, false, true, true, true, true,
];

/// What the masters' attribute setters read beyond the master.
#[derive(Clone, Copy)]
pub struct AttributeWorld<'a> {
    /// The world's sprite table (`+0xc0`, its count `+0xd0`): a sprite attribute takes an id only
    /// where the table holds a sprite (`FUN_14152e360`, `FUN_141533c70`).
    pub sprite: &'a dyn Fn(i32) -> bool,
    /// The player's handicap's attack factor (`player+0x668`, its `+0x14`), None without one.
    pub handicap: Option<f32>,
    /// The player's `+0x648`, which an add or a multiply passes: hit points and carry capacity
    /// stop at 32767, the speed at its cap.
    pub capped: bool,
}

/// The sprite table shows as its absence of a name.
impl std::fmt::Debug for AttributeWorld<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AttributeWorld")
            .field("handicap", &self.handicap)
            .field("capped", &self.capped)
            .finish_non_exhaustive()
    }
}

/// `CVTTSS2SI` into a 32-bit register, then its low 16 bits.
fn low16(v: f32) -> i16 {
    crate::checkpoint::cvttss2si(v) as i16
}

/// `CVTTSS2SI` into a 32-bit register, then its low byte.
fn low8(v: f32) -> u8 {
    crate::checkpoint::cvttss2si(v) as u8
}

/// A saved id (an `int` field) as the slice holds it: None for one below 0 or beyond a short,
/// as the save's (`volley_missile_id`, `mChargeProjectileUnit`).
fn id_of(id: i32) -> Option<i16> {
    i16::try_from(id).ok().filter(|&id| 0 <= id)
}

/// `FUN_14152e360`: the sprite `id`, when the world's table holds one there; else the field keeps
/// its sprite (the game releases the old one first, which is not simulation).
fn set_sprite(field: &mut i16, id: i32, world: &AttributeWorld<'_>) {
    if (world.sprite)(id) {
        *field = id as i16;
    }
}

/// An add to a sprite attribute: its id (−1 for none) plus the amount (`CVTTSS2SI`, in 32
/// bits), then [`set_sprite`].
fn add_sprite(field: &mut i16, v: f32, world: &AttributeWorld<'_>) {
    let id = i32::from(*field) + crate::checkpoint::cvttss2si(v);
    set_sprite(field, id, world);
}

/// An add to the hit points or the carry capacity (`FUN_141126af0`): capped, a sum of at least
/// 32767 (or NaN, `COMISS` then `JBE`) holds 0x7fff; else the amount's low 16 bits add.
fn add_short(value: i16, v: f32, capped: bool) -> i16 {
    if capped && crate::fmath::at_most_or_unordered(SHORT_CAP, f32::from(value) + v) {
        0x7fff
    } else {
        low16(v).wrapping_add(value)
    }
}

/// A multiply of the hit points or the carry capacity (`FUN_1411271e0`): `value × v + 0.5`,
/// truncated; capped, a product of at least 32767 (or NaN) holds 0x7fff.
fn multiply_short(value: i16, v: f32, capped: bool) -> i16 {
    let x = f32::from(value) * v + ROUND;
    if capped && crate::fmath::at_most_or_unordered(SHORT_CAP, x) {
        0x7fff
    } else {
        low16(x)
    }
}

/// An armor or attack amount's class (its high byte) and amount (its low byte, negated for a
/// negative amount; `COMISS` with 0 then `JC`, so a NaN takes the negative branch).
fn class_amount(v: f32) -> (i16, i16) {
    if crate::fmath::below_or_unordered(v, 0.0) {
        let u = crate::checkpoint::cvttss2si(-v) as u16;
        ((u as i16) >> 8, ((u & 0xff) as i16).wrapping_neg())
    } else {
        let u = crate::checkpoint::cvttss2si(v) as u16;
        ((u as i16) >> 8, (u & 0xff) as i16)
    }
}

/// The handicap raises an attack class (`FUN_1410c12b0`): a factor above 1 (`COMISS`, `JBE`: a
/// NaN does not) for a class above 0x21 (an unsigned compare) or not one of [`UNHANDICAPPED`].
fn handicapped(handicap: Option<f32>, class: i16) -> Option<f32> {
    let f = handicap?;
    let raised = !crate::fmath::at_most_or_unordered(f, HANDICAP_FLOOR)
        && (0x21 < class as u16 || UNHANDICAPPED >> (class as u16 & 0x3f) & 1 == 0);
    raised.then_some(f)
}

/// The resource a cost attribute names (`FUN_1412661d0`'s 100, 103 to 106): −1 every paid one,
/// then food, wood, gold (3) and stone (2).
const fn cost_resource(attribute: u8) -> i16 {
    match attribute {
        attribute::FOOD_COSTS => 0,
        attribute::WOOD_COSTS => 1,
        attribute::GOLD_COSTS => 3,
        attribute::STONE_COSTS => 2,
        _ => -1,
    }
}

/// `FUN_1410e87c0`: the list takes `count` entries (`MAXSS` with 0, so a NaN stays and becomes
/// the most; to an unsigned 64-bit count, at most `most`), new ones `fill`; the selected entry
/// (`entry`) stays below the count (an unsigned compare: with none left it stays).
fn resize_count(v: f32) -> u64 {
    let f = crate::fmath::maxss(0.0, v);
    // `CVTTSS2SI RBX` plus 2^63 past 2^63: any value from there on is past the most
    crate::checkpoint::cvttss2si_64(f) as u64
}

/// The damage sprites resized to `v` (`FUN_1410e87c0`): new entries are zero, the selected entry
/// (an unsigned byte compare with the count less one) at most the last.
fn resize_damage_graphics(list: &mut Vec<DamageGraphic>, entry: &mut u8, v: f32) {
    let n = resize_count(v).min(MOST_DAMAGE_GRAPHICS) as usize;
    list.resize(n, DamageGraphic::default());
    let last = (n as u8).wrapping_sub(1);
    if last < *entry {
        *entry = last;
    }
}

/// The train locations resized to `v` (`FUN_141249220`): new entries copy the last one (zero
/// for an empty list), the selected entry (a signed short compare with the count less one) at
/// most the last, −1 for none.
fn resize_train_locations(list: &mut Vec<TrainLocation>, entry: &mut i16, v: f32) {
    let n = resize_count(v).min(MOST_TRAIN_LOCATIONS) as usize;
    let fill = list.last().copied().unwrap_or_default();
    list.resize(n, fill);
    let last = (n as i16).wrapping_sub(1);
    if last < *entry {
        *entry = last;
    }
}

/// A multiply of the damage sprites' selected entry (`FUN_1410e8680`): `v` truncated (32 bits);
/// below 0 the entry at its complement (at most the last) goes, else a zero entry comes at it
/// (at most the count; none past 0xfe entries). The selected entry is that place, at most the
/// last (−1, so 0xff, with none left); an empty list or a full one changes nothing.
fn insert_damage_graphic(list: &mut Vec<DamageGraphic>, entry: &mut u8, v: f32) {
    let mut at = crate::checkpoint::cvttss2si(v);
    if at < 0 {
        if list.is_empty() {
            return;
        }
        at = (!at).min(list.len() as i32 - 1);
        let _ = list.remove(at as usize);
    } else {
        if MOST_DAMAGE_GRAPHICS as usize <= list.len() {
            return;
        }
        at = at.min(list.len() as i32);
        list.insert(at as usize, DamageGraphic::default());
    }
    let last = list.len() as i32 - 1;
    *entry = if last < at { last as u8 } else { at as u8 };
}

/// A multiply of the train locations' selected entry (`FUN_141249040`): as
/// [`insert_damage_graphic`], up to 0x7ffe entries, the new one a copy of the last (zero for an
/// empty list).
fn insert_train_location(list: &mut Vec<TrainLocation>, entry: &mut i16, v: f32) {
    let mut at = crate::checkpoint::cvttss2si(v);
    if at < 0 {
        if list.is_empty() {
            return;
        }
        at = (!at).min(list.len() as i32 - 1);
        let _ = list.remove(at as usize);
    } else {
        if MOST_TRAIN_LOCATIONS as usize <= list.len() {
            return;
        }
        at = at.min(list.len() as i32);
        let fill = list.last().copied().unwrap_or_default();
        list.insert(at as usize, fill);
    }
    let last = list.len() as i32 - 1;
    *entry = if last < at { last as i16 } else { at as i16 };
}

/// The selected entry moved to `n` (the attributes 154 and 158): at most the last of `count`
/// (a signed compare), nothing for an empty list.
fn select_entry(count: usize, n: i32) -> Option<i32> {
    (count != 0).then(|| (count as i32 - 1).min(n))
}

impl Master {
    /// Slot 0x18 (`set`): attribute `attribute` of this master takes `v`, by its type's setter
    /// (`FUN_14123d310` for a building, `FUN_1412661d0` a combat master, `FUN_1410d5a20` a
    /// missile, `FUN_1410c12b0` an RGE combat master, `FUN_14109fbb0` an action master,
    /// `FUN_1410e1c00` a moving one, `FUN_1410a8b00` an animated one or a doppelganger,
    /// `FUN_141126400` the rest), each falling back on its base's. Each first records the master
    /// for the spectator (`FUN_1417785e0`, not simulation). The interface's values (icons, names,
    /// tooltips, hotkey actions) and the sounds are not held. The followers of the values
    /// (`Master::follow`) are the caller's.
    pub fn set_attribute(&mut self, attribute: u8, v: f32, world: &AttributeWorld<'_>) {
        match self.kind {
            master_type::BUILDING => self.set_building(attribute, v, world),
            master_type::COMBAT => self.set_tribe_combat(attribute, v, world),
            master_type::MISSILE => self.set_missile(attribute, v, world),
            master_type::RGE_COMBAT => self.set_combat(attribute, v, world),
            master_type::ACTION => self.set_action(attribute, v, world),
            master_type::MOVING => self.set_moving(attribute, v, world),
            master_type::ANIMATED | master_type::DOPPELGANGER => {
                self.set_animated(attribute, v, world);
            }
            _ => self.set_static(attribute, v, world),
        }
    }

    /// Slot 0x20 (`add`): `v` adds to attribute `attribute`, by its type's setter
    /// (`FUN_14123d530`, `FUN_141266a60`, `FUN_1410d5ab0`, `FUN_1410c16e0`, `FUN_14109fdb0`,
    /// `FUN_1410e1da0`, `FUN_1410a8b60`, `FUN_141126af0`), as [`Master::set_attribute`].
    pub fn add_attribute(&mut self, attribute: u8, v: f32, world: &AttributeWorld<'_>) {
        match self.kind {
            master_type::BUILDING => self.add_building(attribute, v, world),
            master_type::COMBAT => self.add_tribe_combat(attribute, v, world),
            master_type::MISSILE => self.add_missile(attribute, v, world),
            master_type::RGE_COMBAT => self.add_combat(attribute, v, world),
            master_type::ACTION => self.add_action(attribute, v, world),
            master_type::MOVING => self.add_moving(attribute, v, world),
            master_type::ANIMATED | master_type::DOPPELGANGER => {
                self.add_animated(attribute, v, world);
            }
            _ => self.add_static(attribute, v, world),
        }
    }

    /// Slot 0x28 (`multiply`): attribute `attribute` is multiplied by `v`, by its type's setter
    /// (`FUN_14123d730`, `FUN_141267040`, `FUN_1410d5b50`, `FUN_1410c1bf0`, `FUN_14109fea0`,
    /// `FUN_1410e1f60`, `FUN_1410a8c00`, `FUN_1411271e0`), as [`Master::set_attribute`].
    pub fn multiply_attribute(&mut self, attribute: u8, v: f32, world: &AttributeWorld<'_>) {
        match self.kind {
            master_type::BUILDING => self.multiply_building(attribute, v, world),
            master_type::COMBAT => self.multiply_tribe_combat(attribute, v, world),
            master_type::MISSILE => self.multiply_missile(attribute, v, world),
            master_type::RGE_COMBAT => self.multiply_combat(attribute, v, world),
            master_type::ACTION => self.multiply_action(attribute, v, world),
            master_type::MOVING => self.multiply_moving(attribute, v, world),
            master_type::ANIMATED | master_type::DOPPELGANGER => {
                self.multiply_animated(attribute, v, world);
            }
            _ => self.multiply_static(attribute, v, world),
        }
    }

    /// The master copy, slot 8 (`FUN_1411252b0` and its overrides up to the building's
    /// `FUN_14123c4e0`), as an upgrade of a master to another of its type runs it: this master
    /// takes `other`'s values (its tasks, costs, train locations, damage sprites and every other
    /// list copied) but keeps its id, its class, its availability, its disabling and its base id
    /// (`+0x260`; its `+0x5ec`, `+0x738` and, keeping its id, the interface's names: not held). The
    /// action master's work rate takes `other`'s times this one's handicapped rate (`+0x79c`)
    /// when the two differ: not held, taken equal.
    pub fn copy_from(&mut self, other: &Self) {
        let (id, class, available, disabled) = (self.id, self.class, self.available, self.disabled);
        let base_id = self.base_id;
        *self = other.clone();
        self.id = id;
        self.base_id = base_id;
        self.class = class;
        self.available = available;
        self.disabled = disabled;
    }

    /// The value attribute `attribute` changes that the masters' loader left at its default
    /// while what the slice derives from it holds another (the speed, `+0x740`, under a moving
    /// master's speeds; the turn speed, `+0x758`, under its turn time; the train locations under
    /// their ids; the damage sprites under the flag that it has some): the change would derive
    /// from the default. None when the master holds it.
    #[must_use]
    pub fn unloaded_for(&self, attribute: u8) -> Option<&'static str> {
        use crate::masters::attribute as a;
        let moving = self.moving.as_ref();
        match attribute {
            a::MOVEMENT_SPEED | a::STANDING_GRAPHIC | a::WALKING_GRAPHIC | a::RUNNING_GRAPHIC
                if self.max_speed == 0.0 && moving.is_some_and(|m| m.speed != 0.0) =>
            {
                Some("a master's speed (+0x740) the loader did not fill")
            }
            a::ROTATION_SPEED
                if self.turn_speed == 0.0 && moving.is_some_and(|m| m.turn_time != 0.0) =>
            {
                Some("a master's turn speed (+0x758) the loader did not fill")
            }
            a::TRAIN_LOCATION
            | a::TRAIN_BUTTON
            | a::HOTKEY_ID
            | a::TRAIN_TIME
            | a::TRAIN_LOCATIONS_ENTRY_MOD
            | a::TRAIN_LOCATIONS_TOTAL_NUM
                if self.train_locations.is_empty() && !self.trained_at.is_empty() =>
            {
                Some("a master's train locations (+0x898) the loader did not fill")
            }
            a::DAMAGE_GRAPHIC
            | a::DAMAGE_GRAPHICS_ENTRY_MOD
            | a::DAMAGE_GRAPHICS_TOTAL_NUM
            | a::DAMAGE_GRAPHIC_PERCENT
            | a::DAMAGE_GRAPHIC_APPLY_MODE
                if self.damage_graphics.is_empty() && self.damage_sprites =>
            {
                Some("a master's damage sprites (+0x638) the loader did not fill")
            }
            _ => None,
        }
    }

    /// The resource storage `attribute` (21, 26, 27) names: the first, second or third entry.
    fn storage_mut(&mut self, attribute: u8) -> Option<&mut f32> {
        let i = match attribute {
            attribute::STORAGE_1 => 0,
            attribute::STORAGE_2 => 1,
            _ => 2,
        };
        self.storage.get_mut(i).map(|e| &mut e.1)
    }

    /// The selected damage sprite (`+0x731` below the count, an unsigned compare).
    fn damage_graphic_mut(&mut self) -> Option<&mut DamageGraphic> {
        self.damage_graphics
            .get_mut(usize::from(self.damage_graphic_entry))
    }

    /// The selected train location (`FUN_1412669d0`: `+0x940` sign-extended, below the count as
    /// an unsigned value, so a negative entry selects none).
    fn train_location_mut(&mut self) -> Option<&mut TrainLocation> {
        let i = usize::try_from(i64::from(self.train_location_entry) as u64).ok()?;
        self.train_locations.get_mut(i)
    }

    /// `FUN_141126400`, the static master's slot 0x18.
    fn set_static(&mut self, attribute: u8, v: f32, world: &AttributeWorld<'_>) {
        use crate::checkpoint::{cvttss2si, cvttss2si_64};
        use crate::masters::attribute as a;
        match attribute {
            a::HIT_POINTS => self.hit_points = low16(v),
            a::LINE_OF_SIGHT => self.line_of_sight = v,
            a::GARRISON_CAPACITY => self.garrison = low8(v),
            a::SIZE_X => self.radius[0] = v,
            a::SIZE_Y => self.radius[1] = v,
            a::CARRY_CAPACITY => self.capacity = low16(v),
            a::STORAGE_1 | a::STORAGE_2 | a::STORAGE_3 => {
                if let Some(amount) = self.storage_mut(attribute) {
                    *amount = f32::from(low16(v));
                }
            }
            a::FOG_VISIBILITY => self.fog_visibility = low8(v),
            a::OCCLUSION_MODE => self.occlusion = low8(v),
            a::SIZE_Z => self.height = v,
            a::CAN_BE_BUILT_ON | a::BLAST_DEFENSE_LEVEL => self.blast_defense = low8(v),
            a::TERRAIN_RESTRICTION_ID => self.terrain_restriction = low16(v),
            // `CVTTSS2SI RAX`, the low 32 bits
            a::UNIT_TRAIT => self.object_flags = cvttss2si_64(v) as u32,
            a::TRAIT_PIECE => {
                self.object_flags = ((cvttss2si_64(v) as u32) << 16) | (self.object_flags & 0xffff);
            }
            a::DEAD_UNIT_ID => self.death_spawns[0] = low16(v),
            a::BLOOD_UNIT => self.death_spawns[1] = low16(v),
            a::STANDING_GRAPHIC => set_sprite(&mut self.standing_sprite, cvttss2si(v), world),
            a::STANDING_GRAPHIC_2 => {
                let mut s = self.standing_sprite_2.unwrap_or(-1);
                set_sprite(&mut s, cvttss2si(v), world);
                self.standing_sprite_2 = (0 <= s).then_some(s);
            }
            a::DYING_GRAPHIC => set_sprite(&mut self.death_sprite, cvttss2si(v), world),
            a::UNDEAD_GRAPHIC => set_sprite(&mut self.undead_sprite, cvttss2si(v), world),
            a::OBSTRUCTION_TYPE => {
                // FUN_141125ae0: its table holds 14 types; a byte past them reads the routine's
                // stack (the game's undefined read, which no .dat effect reaches): kept
                let t = low8(v);
                self.obstruction_type = match VALID_OBSTRUCTIONS.get(usize::from(t)) {
                    Some(false) => 0,
                    _ => t,
                };
            }
            a::BLOCKAGE_CLASS => self.obstruction_class = low8(v),
            a::SELECTION_EFFECT => self.selection_effect = low8(v),
            a::DAMAGE_GRAPHIC => {
                if let Some(g) = self.damage_graphic_mut() {
                    set_sprite(&mut g.sprite, cvttss2si(v), world);
                }
            }
            // the save's byte, which the slice holds as its test against 0
            a::AVAILABLE_UNIT_FLAG => self.available = low8(v) != 0,
            a::DISABLED_UNIT_FLAG => self.disabled = low8(v) != 0,
            a::INTERFACE_KIND => self.unit_level = low8(v),
            a::COMBAT_LEVEL => self.combat_level = low8(v),
            a::INTERACTION_MODE => self.select_level = low8(v),
            a::MINIMAP_MODE => self.map_draw_level = low8(v),
            a::DAMAGE_GRAPHICS_ENTRY_MOD => {
                if let Some(n) = select_entry(self.damage_graphics.len(), cvttss2si(v)) {
                    self.damage_graphic_entry = n as u8;
                }
            }
            a::DAMAGE_GRAPHICS_TOTAL_NUM => {
                resize_damage_graphics(
                    &mut self.damage_graphics,
                    &mut self.damage_graphic_entry,
                    v,
                );
            }
            a::DAMAGE_GRAPHIC_PERCENT => {
                if let Some(g) = self.damage_graphic_mut() {
                    g.percent = low8(v);
                }
            }
            a::DAMAGE_GRAPHIC_APPLY_MODE => {
                if let Some(g) = self.damage_graphic_mut() {
                    g.apply_mode = low8(v);
                }
            }
            // the icon (25), the name and description (50, 51) and the sounds (92 to 99) are the
            // interface's
            _ => {}
        }
    }

    /// `FUN_141126af0`, the static master's slot 0x20.
    fn add_static(&mut self, attribute: u8, v: f32, world: &AttributeWorld<'_>) {
        use crate::checkpoint::{cvttss2si, cvttss2si_64};
        use crate::masters::attribute as a;
        match attribute {
            a::HIT_POINTS => self.hit_points = add_short(self.hit_points, v, world.capped),
            a::LINE_OF_SIGHT => self.line_of_sight += v,
            a::GARRISON_CAPACITY => self.garrison = self.garrison.wrapping_add(low8(v)),
            a::SIZE_X => self.radius[0] += v,
            a::SIZE_Y => self.radius[1] += v,
            a::CARRY_CAPACITY => self.capacity = add_short(self.capacity, v, world.capped),
            a::STORAGE_1 | a::STORAGE_2 | a::STORAGE_3 => {
                if let Some(amount) = self.storage_mut(attribute) {
                    *amount += f32::from(low16(v));
                }
            }
            a::OCCLUSION_MODE => self.occlusion = self.occlusion.wrapping_add(low8(v)),
            a::SIZE_Z => self.height += v,
            a::CAN_BE_BUILT_ON | a::BLAST_DEFENSE_LEVEL => {
                self.blast_defense = self.blast_defense.wrapping_add(low8(v));
            }
            a::UNIT_TRAIT => {
                self.object_flags = self.object_flags.wrapping_add(cvttss2si_64(v) as u32);
            }
            a::STANDING_GRAPHIC => add_sprite(&mut self.standing_sprite, v, world),
            a::STANDING_GRAPHIC_2 => {
                let mut s = self.standing_sprite_2.unwrap_or(-1);
                add_sprite(&mut s, v, world);
                self.standing_sprite_2 = (0 <= s).then_some(s);
            }
            a::DYING_GRAPHIC => add_sprite(&mut self.death_sprite, v, world),
            a::UNDEAD_GRAPHIC => add_sprite(&mut self.undead_sprite, v, world),
            a::DAMAGE_GRAPHIC => {
                if let Some(g) = self.damage_graphic_mut() {
                    add_sprite(&mut g.sprite, v, world);
                }
            }
            a::AVAILABLE_UNIT_FLAG => {
                self.available = u8::from(self.available).wrapping_add(low8(v)) != 0;
            }
            a::DISABLED_UNIT_FLAG => {
                self.disabled = u8::from(self.disabled).wrapping_add(low8(v)) != 0;
            }
            a::INTERFACE_KIND => self.unit_level = self.unit_level.wrapping_add(low8(v)),
            a::COMBAT_LEVEL => self.combat_level = self.combat_level.wrapping_add(low8(v)),
            a::INTERACTION_MODE => self.select_level = self.select_level.wrapping_add(low8(v)),
            a::MINIMAP_MODE => self.map_draw_level = self.map_draw_level.wrapping_add(low8(v)),
            a::DAMAGE_GRAPHICS_ENTRY_MOD => {
                let n = cvttss2si(f32::from(self.damage_graphic_entry) + v);
                if let Some(n) = select_entry(self.damage_graphics.len(), n) {
                    self.damage_graphic_entry = n as u8;
                }
            }
            // the game's: the selected entry plus the amount, not the count
            a::DAMAGE_GRAPHICS_TOTAL_NUM => {
                let count = f32::from(self.damage_graphic_entry) + v;
                resize_damage_graphics(
                    &mut self.damage_graphics,
                    &mut self.damage_graphic_entry,
                    count,
                );
            }
            a::DAMAGE_GRAPHIC_PERCENT => {
                if let Some(g) = self.damage_graphic_mut() {
                    g.percent = g.percent.wrapping_add(low8(v));
                }
            }
            a::DAMAGE_GRAPHIC_APPLY_MODE => {
                if let Some(g) = self.damage_graphic_mut() {
                    g.apply_mode = g.apply_mode.wrapping_add(low8(v));
                }
            }
            // the sounds (92, 94, 96, 98, through slot 0x18) are the interface's
            _ => {}
        }
    }

    /// `FUN_1411271e0`, the static master's slot 0x28.
    fn multiply_static(&mut self, attribute: u8, v: f32, world: &AttributeWorld<'_>) {
        use crate::masters::attribute as a;
        match attribute {
            a::HIT_POINTS => self.hit_points = multiply_short(self.hit_points, v, world.capped),
            a::LINE_OF_SIGHT => self.line_of_sight *= v,
            a::GARRISON_CAPACITY => self.garrison = low8(f32::from(self.garrison) * v + ROUND),
            a::SIZE_X => self.radius[0] *= v,
            a::SIZE_Y => self.radius[1] *= v,
            a::CARRY_CAPACITY => self.capacity = multiply_short(self.capacity, v, world.capped),
            a::STORAGE_1 | a::STORAGE_2 | a::STORAGE_3 => {
                if let Some(amount) = self.storage_mut(attribute) {
                    *amount = f32::from(low16(v * *amount + ROUND));
                }
            }
            a::SIZE_Z => self.height *= v,
            // without the rounding
            a::BLAST_DEFENSE_LEVEL => self.blast_defense = low8(f32::from(self.blast_defense) * v),
            a::AVAILABLE_UNIT_FLAG => {
                self.available = low8(f32::from(u8::from(self.available)) * v + ROUND) != 0;
            }
            a::DAMAGE_GRAPHICS_ENTRY_MOD => {
                insert_damage_graphic(&mut self.damage_graphics, &mut self.damage_graphic_entry, v);
            }
            a::DAMAGE_GRAPHICS_TOTAL_NUM => {
                let count = self.damage_graphics.len() as f32 * v;
                resize_damage_graphics(
                    &mut self.damage_graphics,
                    &mut self.damage_graphic_entry,
                    count,
                );
            }
            _ => {}
        }
    }

    /// The speed's cap (`FUN_1410a8b60`, `FUN_1410a8c00`): capped, a speed above it (`COMISS`,
    /// `JBE`: not a NaN) takes it, 20 for a missile master, else 4.5.
    fn cap_speed(&mut self, capped: bool) {
        let cap = if self.kind == master_type::MISSILE {
            MISSILE_SPEED_CAP
        } else {
            SPEED_CAP
        };
        if capped && !crate::fmath::at_most_or_unordered(self.max_speed, cap) {
            self.max_speed = cap;
        }
    }

    /// `FUN_1410a8b00`, the animated master's slot 0x18.
    fn set_animated(&mut self, attribute: u8, v: f32, world: &AttributeWorld<'_>) {
        if attribute == attribute::MOVEMENT_SPEED {
            self.max_speed = v;
        } else {
            self.set_static(attribute, v, world);
        }
    }

    /// `FUN_1410a8b60`, the animated master's slot 0x20.
    fn add_animated(&mut self, attribute: u8, v: f32, world: &AttributeWorld<'_>) {
        if attribute == attribute::MOVEMENT_SPEED {
            self.max_speed += v;
            self.cap_speed(world.capped);
        } else {
            self.add_static(attribute, v, world);
        }
    }

    /// `FUN_1410a8c00`, the animated master's slot 0x28.
    fn multiply_animated(&mut self, attribute: u8, v: f32, world: &AttributeWorld<'_>) {
        if attribute == attribute::MOVEMENT_SPEED {
            self.max_speed *= v;
            self.cap_speed(world.capped);
        } else {
            self.multiply_static(attribute, v, world);
        }
    }

    /// `FUN_1410e1c00`, the moving master's slot 0x18.
    fn set_moving(&mut self, attribute: u8, v: f32, world: &AttributeWorld<'_>) {
        use crate::checkpoint::cvttss2si;
        use crate::masters::attribute as a;
        match attribute {
            a::ROTATION_SPEED => self.turn_speed = v,
            a::WALKING_GRAPHIC => set_sprite(&mut self.move_sprite, cvttss2si(v), world),
            a::RUNNING_GRAPHIC => set_sprite(&mut self.run_sprite, cvttss2si(v), world),
            a::TRAILING_UNIT => self.trail.unit = low16(v),
            a::TRAIL_MODE => self.trail.mode = low8(v),
            a::TRAIL_DENSITY => self.trail.density = v,
            _ => self.set_animated(attribute, v, world),
        }
    }

    /// `FUN_1410e1da0`, the moving master's slot 0x20.
    fn add_moving(&mut self, attribute: u8, v: f32, world: &AttributeWorld<'_>) {
        use crate::masters::attribute as a;
        match attribute {
            a::ROTATION_SPEED => self.turn_speed += v,
            a::WALKING_GRAPHIC => add_sprite(&mut self.move_sprite, v, world),
            a::RUNNING_GRAPHIC => add_sprite(&mut self.run_sprite, v, world),
            a::TRAILING_UNIT => self.trail.unit = self.trail.unit.wrapping_add(low16(v)),
            a::TRAIL_MODE => self.trail.mode = self.trail.mode.wrapping_add(low8(v)),
            a::TRAIL_DENSITY => self.trail.density += v,
            _ => self.add_animated(attribute, v, world),
        }
    }

    /// `FUN_1410e1f60`, the moving master's slot 0x28.
    fn multiply_moving(&mut self, attribute: u8, v: f32, world: &AttributeWorld<'_>) {
        match attribute {
            attribute::ROTATION_SPEED => self.turn_speed *= v,
            attribute::TRAIL_DENSITY => self.trail.density *= v,
            _ => self.multiply_animated(attribute, v, world),
        }
    }

    /// `FUN_14109fbb0`, the action master's slot 0x18.
    fn set_action(&mut self, attribute: u8, v: f32, world: &AttributeWorld<'_>) {
        match attribute {
            attribute::WORK_RATE => self.work_rate = v,
            attribute::SEARCH_RADIUS => self.search_radius = v,
            attribute::RUN_PATTERN => self.run_pattern = low8(v),
            // the command and move sounds (132 to 135) are the interface's
            attribute::COMMAND_SOUND..=135 => {}
            _ => self.set_moving(attribute, v, world),
        }
    }

    /// `FUN_14109fdb0`, the action master's slot 0x20.
    fn add_action(&mut self, attribute: u8, v: f32, world: &AttributeWorld<'_>) {
        match attribute {
            attribute::WORK_RATE => self.work_rate += v,
            attribute::SEARCH_RADIUS => self.search_radius += v,
            attribute::RUN_PATTERN => self.run_pattern = self.run_pattern.wrapping_add(low8(v)),
            // the command and move sounds (through slot 0x18) are the interface's
            attribute::COMMAND_SOUND | 134 => {}
            _ => self.add_moving(attribute, v, world),
        }
    }

    /// `FUN_14109fea0`, the action master's slot 0x28.
    fn multiply_action(&mut self, attribute: u8, v: f32, world: &AttributeWorld<'_>) {
        match attribute {
            attribute::WORK_RATE => self.work_rate *= v,
            attribute::SEARCH_RADIUS => self.search_radius *= v,
            _ => self.multiply_moving(attribute, v, world),
        }
    }

    /// The armor (`+0x808`) or attack (`+0x818`) entry of `class`, the first in the list.
    fn class_entry(list: &mut [(i16, i16)], class: i16) -> Option<&mut i16> {
        list.iter_mut().find(|e| e.0 == class).map(|e| &mut e.1)
    }

    /// `FUN_1410c12b0`, the RGE combat master's slot 0x18: an armor or attack amount
    /// ([`class_amount`]) replaces its class's; an attack class at 0 the handicap raises
    /// ([`handicapped`]) takes the amount times the factor.
    fn set_combat(&mut self, attribute: u8, v: f32, world: &AttributeWorld<'_>) {
        use crate::checkpoint::cvttss2si;
        use crate::masters::attribute as a;
        match attribute {
            a::ARMOR => {
                let (class, amount) = class_amount(v);
                if let Some(e) = Self::class_entry(&mut self.armors, class) {
                    *e = amount;
                }
            }
            a::ATTACK => {
                let (class, amount) = class_amount(v);
                if let Some(e) = Self::class_entry(&mut self.weapons, class) {
                    *e = match handicapped(world.handicap, class) {
                        Some(f) if *e == 0 => low16(f32::from(amount) * f),
                        _ => amount,
                    };
                }
            }
            a::ATTACK_RELOAD_TIME => self.reload = v,
            a::ACCURACY_PERCENT => self.hit_chance = low16(v),
            a::MAXIMUM_RANGE => self.weapon_range = v,
            a::BASE_ARMOR => self.base_armor = low16(v),
            a::PROJECTILE_UNIT => self.missile = low16(v),
            a::TERRAIN_DEFENSE_BONUS => self.terrain_bonus = low16(v),
            a::MINIMUM_RANGE => self.min_range = v,
            a::BLAST_WIDTH => self.area_effect = v,
            a::BONUS_DAMAGE_RESISTANCE => self.resistance = v,
            a::FRAME_DELAY => self.fire_frame = low16(v),
            a::BLAST_ATTACK_LEVEL => self.blast_level = low8(v),
            a::SHOWN_ATTACK => self.shown.attack = low16(v),
            a::SHOWN_RANGE => self.shown.range = v,
            a::SHOWN_MELEE_ARMOR => self.shown.melee_armor = low16(v),
            a::COMBAT_ABILITY => self.combat_ability = low8(v),
            a::ATTACK_DISPERSION => self.missed_spread = v,
            a::ATTACK_GRAPHIC => set_sprite(&mut self.attack_sprite, cvttss2si(v), world),
            a::AREA_DAMAGE => self.area_damage = v,
            a::DAMAGE_REFLECTION => self.reflection = v,
            a::FRIENDLY_FIRE_DAMAGE => self.friendly_fire = v,
            a::GARRISON_FIREPOWER => self.garrison_firepower = v,
            a::DISPLACEMENT_X..=a::DISPLACEMENT_Z => {
                self.weapon_offset[usize::from(attribute - a::DISPLACEMENT_X)] = v;
            }
            _ => self.set_action(attribute, v, world),
        }
    }

    /// `FUN_1410c16e0`, the RGE combat master's slot 0x20: an armor or attack amount adds to its
    /// class's (an attack class at 0 the handicap raises takes the amount times the factor).
    fn add_combat(&mut self, attribute: u8, v: f32, world: &AttributeWorld<'_>) {
        use crate::masters::attribute as a;
        match attribute {
            a::ROTATION_SPEED => self.turn_speed += v,
            a::ARMOR => {
                let (class, amount) = class_amount(v);
                if let Some(e) = Self::class_entry(&mut self.armors, class) {
                    *e = e.wrapping_add(amount);
                }
            }
            a::ATTACK => {
                let (class, amount) = class_amount(v);
                if let Some(e) = Self::class_entry(&mut self.weapons, class) {
                    *e = match handicapped(world.handicap, class) {
                        Some(f) if *e == 0 => low16(f32::from(amount) * f),
                        _ => e.wrapping_add(amount),
                    };
                }
            }
            a::ATTACK_RELOAD_TIME => self.reload += v,
            a::ACCURACY_PERCENT => self.hit_chance = self.hit_chance.wrapping_add(low16(v)),
            a::MAXIMUM_RANGE => self.weapon_range += v,
            a::BASE_ARMOR => self.base_armor = self.base_armor.wrapping_add(low16(v)),
            a::PROJECTILE_UNIT => self.missile = self.missile.wrapping_add(low16(v)),
            a::TERRAIN_DEFENSE_BONUS => {
                self.terrain_bonus = self.terrain_bonus.wrapping_add(low16(v));
            }
            a::MINIMUM_RANGE => self.min_range += v,
            a::BLAST_WIDTH => self.area_effect += v,
            a::BONUS_DAMAGE_RESISTANCE => self.resistance += v,
            a::FRAME_DELAY => self.fire_frame = self.fire_frame.wrapping_add(low16(v)),
            a::BLAST_ATTACK_LEVEL => self.blast_level = self.blast_level.wrapping_add(low8(v)),
            a::SHOWN_ATTACK => self.shown.attack = self.shown.attack.wrapping_add(low16(v)),
            a::SHOWN_RANGE => self.shown.range += v,
            a::SHOWN_MELEE_ARMOR => {
                self.shown.melee_armor = self.shown.melee_armor.wrapping_add(low16(v));
            }
            a::COMBAT_ABILITY => {
                self.combat_ability = self.combat_ability.wrapping_add(low8(v));
            }
            a::ATTACK_DISPERSION => self.missed_spread += v,
            a::ATTACK_GRAPHIC => add_sprite(&mut self.attack_sprite, v, world),
            a::AREA_DAMAGE => self.area_damage += v,
            a::DAMAGE_REFLECTION => self.reflection += v,
            a::FRIENDLY_FIRE_DAMAGE => self.friendly_fire += v,
            a::GARRISON_FIREPOWER => self.garrison_firepower += v,
            a::DISPLACEMENT_X..=a::DISPLACEMENT_Z => {
                let o = &mut self.weapon_offset[usize::from(attribute - a::DISPLACEMENT_X)];
                *o += v;
            }
            _ => self.add_action(attribute, v, world),
        }
    }

    /// `FUN_1410c1bf0`, the RGE combat master's slot 0x28: an armor or attack amount is a
    /// percentage of its class's (`amount / 100 × value + 0.5`, truncated; an attack class at 0
    /// the handicap raises scales the amount by the factor first).
    fn multiply_combat(&mut self, attribute: u8, v: f32, world: &AttributeWorld<'_>) {
        use crate::masters::attribute as a;
        match attribute {
            a::ROTATION_SPEED => self.turn_speed *= v,
            a::ARMOR => {
                let (class, amount) = class_amount(v);
                if let Some(e) = Self::class_entry(&mut self.armors, class) {
                    *e = low16(f32::from(amount) / PERCENT * f32::from(*e) + ROUND);
                }
            }
            a::ATTACK => {
                let (class, amount) = class_amount(v);
                if let Some(e) = Self::class_entry(&mut self.weapons, class) {
                    let share = match handicapped(world.handicap, class) {
                        Some(f) if *e == 0 => f32::from(amount) * f,
                        _ => f32::from(amount),
                    };
                    *e = low16(f32::from(*e) * (share / PERCENT) + ROUND);
                }
            }
            a::ATTACK_RELOAD_TIME => self.reload *= v,
            a::ACCURACY_PERCENT => self.hit_chance = low16(f32::from(self.hit_chance) * v + ROUND),
            a::MAXIMUM_RANGE => self.weapon_range *= v,
            a::BASE_ARMOR => self.base_armor = low16(f32::from(self.base_armor) * v + ROUND),
            a::PROJECTILE_UNIT => self.missile = low16(f32::from(self.missile) * v + ROUND),
            a::TERRAIN_DEFENSE_BONUS => {
                self.terrain_bonus = low16(f32::from(self.terrain_bonus) * v + ROUND);
            }
            a::MINIMUM_RANGE => self.min_range *= v,
            a::BLAST_WIDTH => self.area_effect *= v,
            a::BONUS_DAMAGE_RESISTANCE => self.resistance *= v,
            // integer products (`IMUL`) of the truncated amount
            a::FRAME_DELAY => self.fire_frame = self.fire_frame.wrapping_mul(low16(v)),
            a::BLAST_ATTACK_LEVEL => self.blast_level = self.blast_level.wrapping_mul(low8(v)),
            a::SHOWN_ATTACK => self.shown.attack = self.shown.attack.wrapping_mul(low16(v)),
            a::SHOWN_RANGE => self.shown.range *= v,
            a::SHOWN_MELEE_ARMOR => {
                self.shown.melee_armor = self.shown.melee_armor.wrapping_mul(low16(v));
            }
            a::ATTACK_DISPERSION => self.missed_spread *= v,
            a::AREA_DAMAGE => self.area_damage *= v,
            a::DAMAGE_REFLECTION => self.reflection *= v,
            a::FRIENDLY_FIRE_DAMAGE => self.friendly_fire *= v,
            a::GARRISON_FIREPOWER => self.garrison_firepower *= v,
            a::DISPLACEMENT_X..=a::DISPLACEMENT_Z => {
                let o = &mut self.weapon_offset[usize::from(attribute - a::DISPLACEMENT_X)];
                *o *= v;
            }
            _ => self.multiply_action(attribute, v, world),
        }
    }

    /// `FUN_1410d5a20`, the missile master's slot 0x18: its own four values, then always the RGE
    /// combat master's setter (which none of them reaches).
    fn set_missile(&mut self, attribute: u8, v: f32, world: &AttributeWorld<'_>) {
        if let Some(m) = self.missile_stats.as_mut() {
            match attribute {
                attribute::PROJECTILE_SMART_MODE => m.smart_mode = low8(v),
                attribute::PROJECTILE_HIT_MODE => m.hit_mode = low8(v),
                attribute::PROJECTILE_VANISH_MODE => m.vanish_mode = low8(v),
                attribute::PROJECTILE_ARC => m.arc = v,
                _ => {}
            }
        }
        self.set_combat(attribute, v, world);
    }

    /// `FUN_1410d5ab0`, the missile master's slot 0x20, as [`Master::set_missile`].
    fn add_missile(&mut self, attribute: u8, v: f32, world: &AttributeWorld<'_>) {
        if let Some(m) = self.missile_stats.as_mut() {
            match attribute {
                attribute::PROJECTILE_SMART_MODE => {
                    m.smart_mode = m.smart_mode.wrapping_add(low8(v));
                }
                attribute::PROJECTILE_HIT_MODE => m.hit_mode = m.hit_mode.wrapping_add(low8(v)),
                attribute::PROJECTILE_VANISH_MODE => {
                    m.vanish_mode = m.vanish_mode.wrapping_add(low8(v));
                }
                attribute::PROJECTILE_ARC => m.arc += v,
                _ => {}
            }
        }
        self.add_combat(attribute, v, world);
    }

    /// `FUN_1410d5b50`, the missile master's slot 0x28, as [`Master::set_missile`]; its smart
    /// mode takes the amount (the game's set, not a product).
    fn multiply_missile(&mut self, attribute: u8, v: f32, world: &AttributeWorld<'_>) {
        if let Some(m) = self.missile_stats.as_mut() {
            match attribute {
                attribute::PROJECTILE_SMART_MODE => m.smart_mode = low8(v),
                attribute::PROJECTILE_ARC => m.arc *= v,
                _ => {}
            }
        }
        self.multiply_combat(attribute, v, world);
    }

    /// The cost entries a cost attribute (100, 103 to 106) changes (`FUN_1412661d0`): those of
    /// its resource, or for 100 the paid ones. The slice holds the entries of a resource (the
    /// game's of −1, which nothing reads, are not held).
    fn costs_of(&mut self, attribute: u8) -> impl Iterator<Item = &mut i16> {
        let resource = cost_resource(attribute);
        self.cost
            .iter_mut()
            .filter(move |c| (c.2 && resource == -1) || c.0 == resource)
            .map(|c| &mut c.1)
    }

    /// `FUN_1412661d0`, the combat master's slot 0x18.
    fn set_tribe_combat(&mut self, attribute: u8, v: f32, world: &AttributeWorld<'_>) {
        use crate::checkpoint::cvttss2si;
        use crate::masters::attribute as a;
        match attribute {
            a::HERO_STATUS => self.hero_flag = low8(v),
            a::TRAIN_LOCATION => {
                if let Some(t) = self.train_location_mut() {
                    t.location = low16(v);
                }
            }
            a::TRAIN_BUTTON => {
                if let Some(t) = self.train_location_mut() {
                    t.button = low8(v);
                }
            }
            a::SHOWN_PIERCE_ARMOR => self.shown.pierce_armor = low16(v),
            a::HOTKEY_ID => {
                if let Some(t) = self.train_location_mut() {
                    t.hotkey = cvttss2si(v);
                }
            }
            a::MAXIMUM_CHARGE => self.max_charge = v,
            a::RECHARGE_RATE => self.recharge_rate = v,
            a::CHARGE_EVENT => self.charge_event = low16(v),
            a::CHARGE_TYPE => self.charge_type = low16(v),
            a::SECONDARY_PROJECTILE_UNIT => self.volley_missile = id_of(cvttss2si(v)),
            a::SPECIAL_GRAPHIC => set_sprite(&mut self.special_sprite, cvttss2si(v), world),
            a::SPECIAL_ABILITY => self.special_attack = low8(v),
            a::IDLE_ATTACK_GRAPHIC => {
                set_sprite(&mut self.idle_attack_sprite, cvttss2si(v), world);
            }
            a::HERO_GLOW_GRAPHIC => set_sprite(&mut self.hero_glow_sprite, cvttss2si(v), world),
            a::GARRISON_GRAPHIC => set_sprite(&mut self.garrison_sprite, cvttss2si(v), world),
            a::RESOURCE_COSTS | a::FOOD_COSTS..=a::STONE_COSTS => {
                for c in self.costs_of(attribute) {
                    *c = low16(v);
                }
            }
            a::TRAIN_TIME => {
                if let Some(t) = self.train_location_mut() {
                    t.time = low16(v);
                }
            }
            a::TOTAL_MISSILES => self.volley_fire = v,
            a::MAXIMUM_TOTAL_MISSILES => self.max_volley = low8(v),
            a::REGENERATION_RATE => self.regeneration = v,
            a::POPULATION => {
                // the unpaid entries take the negated amount; the storage of population (4) the
                // amount, of resources 11 and 19 its negation
                let negated = -v;
                for c in self.cost.iter_mut().filter(|c| !c.2) {
                    c.1 = low16(negated);
                }
                for s in &mut self.storage {
                    match s.0 {
                        4 => s.1 = v,
                        11 | 19 => s.1 = negated,
                        _ => {}
                    }
                }
            }
            a::MIN_CONVERSION_TIME_MODIFIER..=a::CONVERSION_CHANCE_MODIFIER => {
                self.conversion[usize::from(attribute - a::MIN_CONVERSION_TIME_MODIFIER)] = v;
            }
            a::FORMATION_CATEGORY => self.formation_category = low8(v),
            a::REGENERATION_HP_PERCENT => self.regeneration = f32::from(self.hit_points) * v,
            a::CHARGE_PROJECTILE_UNIT => self.charge_projectile = id_of(cvttss2si(v)),
            a::ATTACK_PRIORITY => self.attack_priority = low8(v),
            a::INVULNERABILITY_LEVEL => self.invulnerability = v,
            a::ATTACK_GRAPHIC_2 => set_sprite(&mut self.attack_sprite2, cvttss2si(v), world),
            a::SPAWNING_AREA_WIDTH..=a::SPAWNING_AREA_RANDOMNESS => {
                self.volley_spread[usize::from(attribute - a::SPAWNING_AREA_WIDTH)] = v;
            }
            a::TRAIN_LOCATIONS_ENTRY_MOD => {
                if let Some(n) = select_entry(self.train_locations.len(), cvttss2si(v)) {
                    self.train_location_entry = n as i16;
                }
            }
            a::TRAIN_LOCATIONS_TOTAL_NUM => {
                resize_train_locations(
                    &mut self.train_locations,
                    &mut self.train_location_entry,
                    v,
                );
            }
            // the ability's button, tooltips and hotkey action are the interface's
            a::ABILITY_BUTTON_ICON_ID..=a::ABILITY_HOTKEY_ACTION => {}
            _ => self.set_combat(attribute, v, world),
        }
    }

    /// `FUN_141266a60`, the combat master's slot 0x20. Its sprite adds read the sprite's id
    /// without the others' test for none (a crash on none, which no recording reaches): none
    /// counts as −1.
    fn add_tribe_combat(&mut self, attribute: u8, v: f32, world: &AttributeWorld<'_>) {
        use crate::checkpoint::cvttss2si;
        use crate::masters::attribute as a;
        match attribute {
            a::HERO_STATUS => self.hero_flag = self.hero_flag.wrapping_add(low8(v)),
            a::SHOWN_PIERCE_ARMOR => {
                self.shown.pierce_armor = self.shown.pierce_armor.wrapping_add(low16(v));
            }
            a::MAXIMUM_CHARGE => self.max_charge += v,
            a::RECHARGE_RATE => self.recharge_rate += v,
            a::CHARGE_EVENT => self.charge_event = self.charge_event.wrapping_add(low16(v)),
            a::SPECIAL_GRAPHIC => add_sprite(&mut self.special_sprite, v, world),
            a::SPECIAL_ABILITY => self.special_attack = self.special_attack.wrapping_add(low8(v)),
            a::IDLE_ATTACK_GRAPHIC => add_sprite(&mut self.idle_attack_sprite, v, world),
            a::HERO_GLOW_GRAPHIC => add_sprite(&mut self.hero_glow_sprite, v, world),
            a::GARRISON_GRAPHIC => add_sprite(&mut self.garrison_sprite, v, world),
            a::RESOURCE_COSTS | a::FOOD_COSTS..=a::STONE_COSTS => {
                for c in self.costs_of(attribute) {
                    *c = c.wrapping_add(low16(v));
                }
            }
            a::TRAIN_TIME => {
                if let Some(t) = self.train_location_mut() {
                    t.time = t.time.wrapping_add(low16(v));
                }
            }
            a::TOTAL_MISSILES => self.volley_fire += v,
            a::MAXIMUM_TOTAL_MISSILES => self.max_volley = self.max_volley.wrapping_add(low8(v)),
            a::REGENERATION_RATE => self.regeneration += v,
            a::MIN_CONVERSION_TIME_MODIFIER..=a::CONVERSION_CHANCE_MODIFIER => {
                let c =
                    &mut self.conversion[usize::from(attribute - a::MIN_CONVERSION_TIME_MODIFIER)];
                *c += v;
            }
            a::REGENERATION_HP_PERCENT => {
                self.regeneration += f32::from(self.hit_points) * v;
            }
            a::INVULNERABILITY_LEVEL => self.invulnerability += v,
            // FUN_141533c70 tests the new id; the game takes the sprite from the same table
            a::ATTACK_GRAPHIC_2 => add_sprite(&mut self.attack_sprite2, v, world),
            a::SPAWNING_AREA_WIDTH..=a::SPAWNING_AREA_RANDOMNESS => {
                let s = &mut self.volley_spread[usize::from(attribute - a::SPAWNING_AREA_WIDTH)];
                *s += v;
            }
            a::TRAIN_LOCATIONS_ENTRY_MOD => {
                let n = cvttss2si(f32::from(self.train_location_entry) + v);
                if let Some(n) = select_entry(self.train_locations.len(), n) {
                    self.train_location_entry = n as i16;
                }
            }
            // the game's: the selected entry plus the amount, not the count
            a::TRAIN_LOCATIONS_TOTAL_NUM => {
                let count = f32::from(self.train_location_entry) + v;
                resize_train_locations(
                    &mut self.train_locations,
                    &mut self.train_location_entry,
                    count,
                );
            }
            a::ABILITY_BUTTON_ICON_ID..=a::ABILITY_HOTKEY_ACTION => {}
            _ => self.add_combat(attribute, v, world),
        }
    }

    /// `FUN_141267040`, the combat master's slot 0x28.
    fn multiply_tribe_combat(&mut self, attribute: u8, v: f32, world: &AttributeWorld<'_>) {
        use crate::masters::attribute as a;
        match attribute {
            a::SHOWN_PIERCE_ARMOR => {
                self.shown.pierce_armor = self.shown.pierce_armor.wrapping_mul(low16(v));
            }
            a::MAXIMUM_CHARGE => self.max_charge *= v,
            a::RECHARGE_RATE => self.recharge_rate *= v,
            a::RESOURCE_COSTS | a::FOOD_COSTS..=a::STONE_COSTS => {
                for c in self.costs_of(attribute) {
                    *c = low16(f32::from(*c) * v + ROUND);
                }
            }
            a::TRAIN_TIME => {
                // FUN_140ace5e0 reads 0 past the list; FUN_14126ade0 writes only within it
                if let Some(t) = self.train_location_mut() {
                    t.time = low16(f32::from(t.time) * v + ROUND);
                }
            }
            a::TOTAL_MISSILES => self.volley_fire *= v,
            a::MAXIMUM_TOTAL_MISSILES => {
                self.max_volley = low8(f32::from(self.max_volley) * v + ROUND);
            }
            a::REGENERATION_RATE => self.regeneration *= v,
            a::MIN_CONVERSION_TIME_MODIFIER..=a::CONVERSION_CHANCE_MODIFIER => {
                let c =
                    &mut self.conversion[usize::from(attribute - a::MIN_CONVERSION_TIME_MODIFIER)];
                *c *= v;
            }
            a::INVULNERABILITY_LEVEL => self.invulnerability *= v,
            a::SPAWNING_AREA_WIDTH..=a::SPAWNING_AREA_RANDOMNESS => {
                let s = &mut self.volley_spread[usize::from(attribute - a::SPAWNING_AREA_WIDTH)];
                *s *= v;
            }
            a::TRAIN_LOCATIONS_ENTRY_MOD => {
                insert_train_location(&mut self.train_locations, &mut self.train_location_entry, v);
            }
            a::TRAIN_LOCATIONS_TOTAL_NUM => {
                let count = self.train_locations.len() as f32 * v;
                resize_train_locations(
                    &mut self.train_locations,
                    &mut self.train_location_entry,
                    count,
                );
            }
            _ => self.multiply_combat(attribute, v, world),
        }
    }

    /// `FUN_14123d310`, the building master's slot 0x18. Its six sprites (85 to 90) go to a copy
    /// of the field (the game's local), which changes nothing; its sounds are the interface's.
    fn set_building(&mut self, attribute: u8, v: f32, world: &AttributeWorld<'_>) {
        use crate::masters::attribute as a;
        match attribute {
            a::BUILDING_ICON_OVERRIDE => self.building_facet = low16(v),
            a::GARRISON_TYPE => {
                if let Some(b) = self.building.as_mut() {
                    b.garrison_type = low8(v);
                }
            }
            a::FOUNDATION_TERRAIN => {
                if let Some(b) = self.building.as_mut() {
                    b.foundation_terrain = low16(v);
                }
            }
            a::GARRISON_HEAL_RATE => {
                if let Some(b) = self.building.as_mut() {
                    b.garrison_heal_rate = v;
                }
            }
            a::CONSTRUCTION_GRAPHIC..=a::RESEARCH_COMPLETED_GRAPHIC
            | 136..=a::TRANSFORM_SOUND_EVENT => {}
            _ => self.set_tribe_combat(attribute, v, world),
        }
    }

    /// `FUN_14123d530`, the building master's slot 0x20, as [`Master::set_building`].
    fn add_building(&mut self, attribute: u8, v: f32, world: &AttributeWorld<'_>) {
        use crate::masters::attribute as a;
        match attribute {
            a::BUILDING_ICON_OVERRIDE => {
                self.building_facet = self.building_facet.wrapping_add(low16(v));
            }
            a::GARRISON_TYPE => {
                if let Some(b) = self.building.as_mut() {
                    b.garrison_type = b.garrison_type.wrapping_add(low8(v));
                }
            }
            a::FOUNDATION_TERRAIN => {
                if let Some(b) = self.building.as_mut() {
                    b.foundation_terrain = b.foundation_terrain.wrapping_add(low16(v));
                }
            }
            a::GARRISON_HEAL_RATE => {
                if let Some(b) = self.building.as_mut() {
                    b.garrison_heal_rate += v;
                }
            }
            a::CONSTRUCTION_GRAPHIC..=a::RESEARCH_COMPLETED_GRAPHIC | 136 | 138 => {}
            _ => self.add_tribe_combat(attribute, v, world),
        }
    }

    /// `FUN_14123d730`, the building master's slot 0x28.
    fn multiply_building(&mut self, attribute: u8, v: f32, world: &AttributeWorld<'_>) {
        match (attribute, self.building.as_mut()) {
            (attribute::GARRISON_HEAL_RATE, Some(b)) => {
                b.garrison_heal_rate *= v;
            }
            (attribute::GARRISON_HEAL_RATE, None) => {}
            _ => self.multiply_tribe_combat(attribute, v, world),
        }
    }
}

/// A sprite's speed factor (the `.dat` graphic's `speed_multiplier`): the master's speed times
/// it when above 0, else the master's speed (as the masters' loader derives
/// [`MoveStats`]' speeds).
fn sprite_speed(
    base: f32,
    sprite: i16,
    graphics: &std::collections::BTreeMap<i16, crate::sprite::Graphic>,
) -> f32 {
    let factor = graphics.get(&sprite).map_or(0.0, |g| g.speed_multiplier);
    if 0.0 < factor { base * factor } else { base }
}

impl Master {
    /// The facets the facing routines read of a unit of this master ([`MoveStats::facets`]):
    /// those of its standing sprites, its moving ones (types 30 and up), its attack sprite (50
    /// and up) and its tasks' sprites, when every one the graphics hold has the same count and
    /// bit 1 (as the masters' loader derives them).
    #[must_use]
    pub fn sprite_facets(
        &self,
        graphics: &std::collections::BTreeMap<i16, crate::sprite::Graphic>,
    ) -> Option<crate::facing::SpriteFacets> {
        let mut sprites = vec![self.standing_sprite, self.standing_sprite_2.unwrap_or(-1)];
        if master_type::MOVING <= self.kind {
            sprites.extend([self.move_sprite, self.run_sprite]);
        }
        if master_type::RGE_COMBAT <= self.kind {
            sprites.push(self.attack_sprite);
        }
        for t in &self.tasks {
            sprites.extend(t.sprites);
        }
        let mut seen = sprites
            .into_iter()
            .filter(|&id| 0 <= id)
            .filter_map(|id| graphics.get(&id))
            .map(|g| crate::facing::SpriteFacets {
                count: g.facets,
                flag_1: g.flags & 2 != 0,
            });
        let first = seen.next()?;
        seen.all(|f| f == first).then_some(first)
    }

    /// What a unit moving with this master reads after attribute `attribute` changed
    /// (`stats`, the master's [`Master::moving`] or a unit's copy): in the game the unit reads its
    /// master, so its radii, speeds, turn time, terrain restriction (its mask from `terrain`,
    /// kept without one), fast turn, weapon range and facets follow the master's values.
    pub fn follow_moving(
        &self,
        stats: &mut MoveStats,
        attribute: u8,
        graphics: &std::collections::BTreeMap<i16, crate::sprite::Graphic>,
        terrain: Option<&crate::terrain::Terrain>,
    ) {
        use crate::masters::attribute as a;
        match attribute {
            a::SIZE_X | a::SIZE_Y | a::SIZE_Z => {
                stats.radius = [self.radius[0], self.radius[1], self.height];
            }
            a::MOVEMENT_SPEED | a::STANDING_GRAPHIC | a::WALKING_GRAPHIC | a::RUNNING_GRAPHIC => {
                let run = if 0 <= self.run_sprite {
                    self.run_sprite
                } else {
                    self.move_sprite
                };
                stats.speed = sprite_speed(self.max_speed, run, graphics);
                stats.run_speed = stats.speed;
                stats.walk_speed = sprite_speed(self.max_speed, self.move_sprite, graphics);
                stats.stand_speed = sprite_speed(self.max_speed, self.standing_sprite, graphics);
            }
            a::ROTATION_SPEED => stats.turn_time = self.turn_speed,
            a::TERRAIN_RESTRICTION_ID => {
                // a restriction below 0 has no mask; the stats keep theirs
                if let Ok(r) = usize::try_from(self.terrain_restriction) {
                    stats.restriction = r;
                    if let Some(t) = terrain {
                        stats.restriction_mask = t.restriction_mask(r);
                    }
                }
            }
            a::UNIT_TRAIT | a::TRAIT_PIECE => stats.fast_turn = self.object_flags & 2 != 0,
            a::MAXIMUM_RANGE => stats.weapon_range = self.weapon_range,
            _ => {}
        }
        if matches!(
            attribute,
            a::STANDING_GRAPHIC
                | a::STANDING_GRAPHIC_2
                | a::WALKING_GRAPHIC
                | a::RUNNING_GRAPHIC
                | a::ATTACK_GRAPHIC
        ) {
            stats.facets = self.sprite_facets(graphics);
        }
    }

    /// The values the slice derives from this master's after attribute `attribute` changed: its
    /// height test, its missile's flight, whether it has damage sprites and a special graphic,
    /// its train locations' ids and first time, and what a unit moving with it reads
    /// ([`Master::follow_moving`]).
    pub fn follow(
        &mut self,
        attribute: u8,
        graphics: &std::collections::BTreeMap<i16, crate::sprite::Graphic>,
        terrain: Option<&crate::terrain::Terrain>,
    ) {
        use crate::masters::attribute as a;
        match attribute {
            a::SIZE_X | a::SIZE_Y | a::SIZE_Z => {
                self.tall = 0.0 < self.height;
                if let Some(m) = self.missile_stats.as_mut() {
                    m.radius = [self.radius[0], self.radius[1], self.height];
                }
            }
            a::MOVEMENT_SPEED => {
                if let Some(m) = self.missile_stats.as_mut() {
                    m.speed = self.max_speed;
                }
            }
            a::DAMAGE_GRAPHICS_ENTRY_MOD | a::DAMAGE_GRAPHICS_TOTAL_NUM => {
                self.damage_sprites = !self.damage_graphics.is_empty();
            }
            a::SPECIAL_GRAPHIC => self.special_graphic = 0 <= self.special_sprite,
            a::TRAIN_LOCATION
            | a::TRAIN_TIME
            | a::TRAIN_LOCATIONS_ENTRY_MOD
            | a::TRAIN_LOCATIONS_TOTAL_NUM => {
                self.trained_at = self.train_locations.iter().map(|t| t.location).collect();
                self.build_time = self.train_locations.first().map_or(0, |t| t.time);
            }
            _ => {}
        }
        if let Some(mut stats) = self.moving.take() {
            self.follow_moving(&mut stats, attribute, graphics, terrain);
            self.moving = Some(stats);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(action_type: i16, object_group: i16, object_id: i16, work_val1: f32) -> Task {
        Task {
            action_type,
            object_group,
            object_id,
            work_val1,
            ..Task::default()
        }
    }

    fn master(id: i16, task_by_group: u8, tasks: Vec<Task>) -> Master {
        Master {
            id,
            kind: 70,
            class: 4,
            task_by_group,
            radius: [0.2, 0.2],
            tall: true,
            obstruction_type: 0,
            buildable_on: 0,
            tasks,
            cost: vec![],
            obstruction_class: 0,
            object_flags: 0,
            combat_level: 0,
            work_rate: 1.0,
            hit_points: 25,
            build_time: 0,
            ..Master::default()
        }
    }

    fn house() -> Master {
        Master {
            id: 70,
            kind: 80,
            class: 3,
            task_by_group: 0,
            radius: [1.0, 1.0],
            tall: true,
            obstruction_type: 0,
            buildable_on: 0,
            tasks: vec![],
            cost: vec![(1, 25, true)],
            obstruction_class: 2,
            object_flags: 0,
            combat_level: 0,
            work_rate: 0.0,
            hit_points: 550,
            build_time: 25,
            ..Master::default()
        }
    }

    /// The villagers of build 158041's `.dat`, shortened: the base masters have no Build task, the
    /// builders a generic one; a fisherman (56) of the male group has none either.
    fn villagers() -> Masters {
        let base = || {
            vec![
                task(7, -1, -1, 0.0),
                task(3, 3, -1, 0.0),
                task(110, -1, 126, 0.0),
            ]
        };
        let builder = || vec![task(101, -1, -1, 1.0), task(3, 3, -1, 0.0)];
        Masters::new([
            master(56, 1, vec![task(5, -1, 53, 1.0)]),
            master(83, 1, base()),
            master(118, 1, builder()),
            master(212, 2, builder()),
            master(293, 2, base()),
            house(),
        ])
    }

    /// Every player on its own, no drop sites.
    struct Alone;

    impl Diplomacy for Alone {
        fn allied(&self, from: usize, to: usize, _: bool) -> bool {
            from == to
        }

        fn drops_at(&self, _: i16, _: &Actor<'_>) -> bool {
            false
        }
    }

    /// A world whose sprite table holds the ids 1 to 99, without a handicap.
    fn world(capped: bool) -> AttributeWorld<'static> {
        AttributeWorld {
            sprite: &|id| (1..100).contains(&id),
            handicap: None,
            capped,
        }
    }

    fn of_type(kind: u8) -> Master {
        Master {
            kind,
            ..Master::default()
        }
    }

    /// `FUN_141126400`, `FUN_141126af0` and `FUN_1411271e0` on the hit points: a set truncates, an
    /// add adds the truncated amount (wrapping), a multiply rounds by 0.5; capped (the player's
    /// `+0x648`) both stop at 0x7fff, a NaN too (`COMISS` then `JBE`).
    #[test]
    fn the_hit_points_add_and_multiply_up_to_their_cap_when_capped() {
        let mut m = of_type(master_type::STATIC);
        m.set_attribute(attribute::HIT_POINTS, 25.9, &world(false));
        assert_eq!(m.hit_points, 25);
        m.add_attribute(attribute::HIT_POINTS, 15.7, &world(false));
        assert_eq!(m.hit_points, 40);
        m.multiply_attribute(attribute::HIT_POINTS, 1.2, &world(false));
        // 40 × 1.2 + 0.5 = 48.5
        assert_eq!(m.hit_points, 48);
        m.hit_points = 32760;
        m.add_attribute(attribute::HIT_POINTS, 10.0, &world(false));
        assert_eq!(m.hit_points, -32766);
        m.hit_points = 32760;
        m.add_attribute(attribute::HIT_POINTS, 10.0, &world(true));
        assert_eq!(m.hit_points, 0x7fff);
        m.hit_points = 100;
        m.add_attribute(attribute::HIT_POINTS, f32::NAN, &world(true));
        assert_eq!(m.hit_points, 0x7fff);
        m.hit_points = 20000;
        m.multiply_attribute(attribute::HIT_POINTS, 2.0, &world(true));
        assert_eq!(m.hit_points, 0x7fff);
        m.capacity = 10;
        m.multiply_attribute(attribute::CARRY_CAPACITY, 1.25, &world(true));
        // 12.5 + 0.5 = 13
        assert_eq!(m.capacity, 13);
    }

    /// The storage amounts take a short (`(float)(int)(short)(int)v`), the object flags a 64-bit
    /// truncation, the trait piece its high half; an obstruction type the table refuses is 0.
    #[test]
    fn the_static_setters_truncate_as_the_game_does() {
        let mut m = of_type(master_type::TREE);
        m.storage = vec![(15, 100.0, 1), (-1, 0.0, 0), (-1, 0.0, 0)];
        m.set_attribute(attribute::STORAGE_1, 70000.5, &world(false));
        assert_eq!(m.storage[0].1, 4464.0);
        m.add_attribute(attribute::STORAGE_1, 5.9, &world(false));
        assert_eq!(m.storage[0].1, 4469.0);
        m.multiply_attribute(attribute::STORAGE_1, 0.5, &world(false));
        // 2234.5 + 0.5 = 2235
        assert_eq!(m.storage[0].1, 2235.0);
        m.set_attribute(attribute::UNIT_TRAIT, 5_000_000_000.0, &world(false));
        assert_eq!(m.object_flags, 5_000_000_000_i64 as u32);
        m.object_flags = 0x1234_5678;
        m.set_attribute(attribute::TRAIT_PIECE, 3.0, &world(false));
        assert_eq!(m.object_flags, 0x0003_5678);
        m.set_attribute(attribute::OBSTRUCTION_TYPE, 5.0, &world(false));
        assert_eq!(m.obstruction_type, 5);
        m.set_attribute(attribute::OBSTRUCTION_TYPE, 4.0, &world(false));
        assert_eq!(m.obstruction_type, 0);
        // "Can Be Built On" writes the blast defense level; its multiply does not round
        m.set_attribute(attribute::CAN_BE_BUILT_ON, 3.0, &world(false));
        m.multiply_attribute(attribute::BLAST_DEFENSE_LEVEL, 1.9, &world(false));
        assert_eq!(m.blast_defense, 5);
        // the available flag holds the byte's test against 0
        m.set_attribute(attribute::AVAILABLE_UNIT_FLAG, 256.0, &world(false));
        assert!(!m.available);
        m.add_attribute(attribute::AVAILABLE_UNIT_FLAG, 1.0, &world(false));
        assert!(m.available);
    }

    /// `FUN_14152e360`: a sprite attribute takes an id only where the world's table holds a
    /// sprite; an add starts from −1 for none.
    #[test]
    fn a_sprite_attribute_takes_only_a_sprite_the_world_has() {
        let mut m = of_type(master_type::MOVING);
        m.standing_sprite = 5;
        m.move_sprite = -1;
        m.set_attribute(attribute::STANDING_GRAPHIC, 150.0, &world(false));
        assert_eq!(m.standing_sprite, 5);
        m.add_attribute(attribute::STANDING_GRAPHIC, 3.0, &world(false));
        assert_eq!(m.standing_sprite, 8);
        m.add_attribute(attribute::WALKING_GRAPHIC, 11.0, &world(false));
        assert_eq!(m.move_sprite, 10);
        m.set_attribute(attribute::STANDING_GRAPHIC_2, 7.0, &world(false));
        assert_eq!(m.standing_sprite_2, Some(7));
    }

    /// `FUN_1410a8b60`, `FUN_1410a8c00`: a capped add or multiply of the speed stops at 4.5, a
    /// missile master's at 20; a NaN speed passes (`COMISS`, `JBE`).
    #[test]
    fn a_capped_speed_stops_at_its_type_s_cap() {
        let mut m = of_type(master_type::COMBAT);
        m.max_speed = 4.0;
        m.add_attribute(attribute::MOVEMENT_SPEED, 1.0, &world(false));
        assert_eq!(m.max_speed, 5.0);
        m.multiply_attribute(attribute::MOVEMENT_SPEED, 1.1, &world(true));
        assert_eq!(m.max_speed, 4.5);
        let mut missile = of_type(master_type::MISSILE);
        missile.max_speed = 7.0;
        missile.multiply_attribute(attribute::MOVEMENT_SPEED, 4.0, &world(true));
        assert_eq!(missile.max_speed, 20.0);
        missile.add_attribute(attribute::MOVEMENT_SPEED, f32::NAN, &world(true));
        assert!(missile.max_speed.is_nan());
    }

    /// `FUN_1410c12b0` and its add and multiply: the amount's high byte names the class, its low
    /// byte the amount (negated for a negative amount); the first entry of the class changes, a
    /// multiply by `amount / 100`.
    #[test]
    fn armor_and_attack_change_the_class_in_the_amount_s_high_byte() {
        let mut m = of_type(master_type::COMBAT);
        m.armors = vec![(4, 1), (3, 2), (3, 9)];
        m.weapons = vec![(4, 6), (3, 0)];
        // class 3, 1 more (0x301)
        m.add_attribute(attribute::ARMOR, 769.0, &world(false));
        assert_eq!(m.armors, vec![(4, 1), (3, 3), (3, 9)]);
        // −0x402: class 4, 2 less
        m.add_attribute(attribute::ARMOR, -1026.0, &world(false));
        assert_eq!(m.armors[0], (4, -1));
        m.set_attribute(attribute::ARMOR, 1029.0, &world(false));
        assert_eq!(m.armors[0], (4, 5));
        // 150 %: 6 × 1.5 + 0.5 = 9.5
        m.multiply_attribute(attribute::ATTACK, f32::from(0x496_i16), &world(false));
        assert_eq!(m.weapons[0], (4, 9));
        // a NaN takes the negative branch: class 0, amount 0, which no entry has
        m.add_attribute(attribute::ATTACK, f32::NAN, &world(false));
        assert_eq!(m.weapons, vec![(4, 9), (3, 0)]);
        // a handicap of 1.5 raises a class at 0 (not 3, 4, 31 or 33)
        let raised = AttributeWorld {
            handicap: Some(1.5),
            ..world(false)
        };
        m.weapons = vec![(4, 0), (21, 0)];
        m.set_attribute(attribute::ATTACK, f32::from(0x406_i16), &raised);
        m.set_attribute(attribute::ATTACK, f32::from(0x1506_i16), &raised);
        assert_eq!(m.weapons, vec![(4, 6), (21, 9)]);
        // once above 0 the add adds
        m.add_attribute(attribute::ATTACK, f32::from(0x1502_i16), &raised);
        assert_eq!(m.weapons[1], (21, 11));
    }

    /// The RGE combat master's multiply rounds most shorts by 0.5 but multiplies the frame delay
    /// and the shown values by the truncated amount (`IMUL`).
    #[test]
    fn some_combat_shorts_multiply_by_the_truncated_amount() {
        let mut m = of_type(master_type::RGE_COMBAT);
        m.base_armor = 3;
        m.fire_frame = 5;
        m.shown.attack = 7;
        m.multiply_attribute(attribute::BASE_ARMOR, 1.5, &world(false));
        assert_eq!(m.base_armor, 5);
        m.multiply_attribute(attribute::FRAME_DELAY, 2.9, &world(false));
        assert_eq!(m.fire_frame, 10);
        m.multiply_attribute(attribute::SHOWN_ATTACK, 3.5, &world(false));
        assert_eq!(m.shown.attack, 21);
        m.add_attribute(attribute::ROTATION_SPEED, 0.25, &world(false));
        assert_eq!(m.turn_speed, 0.25);
        m.set_attribute(attribute::DISPLACEMENT_X + 1, 0.75, &world(false));
        assert_eq!(m.weapon_offset, [0.0, 0.75, 0.0]);
    }

    /// `FUN_1410d5b50`: a missile master's smart mode takes the multiply's amount; its arc
    /// multiplies; then the RGE combat master's setter runs.
    #[test]
    fn a_missile_s_smart_mode_multiply_sets_it() {
        let mut m = of_type(master_type::MISSILE);
        m.missile_stats = Some(crate::missile::MissileStats {
            projectile_type: 0,
            smart_mode: 1,
            hit_mode: 0,
            vanish_mode: 0,
            arc: 0.5,
            speed: 7.0,
            radius: [0.0; 3],
            fly_mode: 0,
        });
        m.reload = 2.0;
        m.multiply_attribute(attribute::PROJECTILE_SMART_MODE, 3.0, &world(false));
        m.multiply_attribute(attribute::PROJECTILE_ARC, 0.5, &world(false));
        m.multiply_attribute(attribute::ATTACK_RELOAD_TIME, 0.5, &world(false));
        let s = m.missile_stats.unwrap();
        assert_eq!((s.smart_mode, s.arc, m.reload), (3, 0.25, 1.0));
    }

    /// `FUN_1412661d0` and its add and multiply on the costs: 100 changes the paid entries, 103
    /// to 106 those of food, wood, gold (3) and stone (2); population (110) sets the unpaid ones
    /// to the negated amount and the population storage to the amount.
    #[test]
    fn the_cost_attributes_change_their_resource_s_entries() {
        let mut m = of_type(master_type::BUILDING);
        m.cost = vec![(1, 25, true), (4, 1, false)];
        m.storage = vec![(4, 5.0, 4), (11, -1.0, 4), (-1, 0.0, 0)];
        m.add_attribute(attribute::WOOD_COSTS, -5.0, &world(false));
        assert_eq!(m.cost, vec![(1, 20, true), (4, 1, false)]);
        m.multiply_attribute(attribute::RESOURCE_COSTS, 0.85, &world(false));
        // 20 × 0.85 + 0.5 = 17.5
        assert_eq!(m.cost, vec![(1, 17, true), (4, 1, false)]);
        m.set_attribute(attribute::GOLD_COSTS, 50.0, &world(false));
        assert_eq!(m.cost[0], (1, 17, true));
        m.set_attribute(attribute::POPULATION, 2.0, &world(false));
        assert_eq!(m.cost, vec![(1, 17, true), (4, -2, false)]);
        assert_eq!(m.storage[..2], [(4, 2.0, 4), (11, -2.0, 4)]);
    }

    /// The train locations: the selected entry (`+0x940`) takes the train time; a resize copies
    /// the last entry, its add counts from the selected entry (the game's); a multiply of the
    /// selected entry inserts a copy of the last or removes one at the amount's complement.
    #[test]
    fn the_train_location_attributes_change_the_selected_entry() {
        let barracks = |time| TrainLocation {
            location: 12,
            time,
            button: 1,
            hotkey: 16_000,
        };
        let mut m = of_type(master_type::COMBAT);
        m.train_locations = vec![barracks(21), barracks(30)];
        m.train_location_entry = 1;
        m.multiply_attribute(attribute::TRAIN_TIME, 0.9, &world(false));
        // 30 × 0.9 + 0.5 = 27.5
        assert_eq!(m.train_locations[1].time, 27);
        m.set_attribute(attribute::TRAIN_LOCATIONS_ENTRY_MOD, 9.0, &world(false));
        assert_eq!(m.train_location_entry, 1);
        // the add's count is the selected entry plus the amount: 1 + 2
        m.add_attribute(attribute::TRAIN_LOCATIONS_TOTAL_NUM, 2.0, &world(false));
        assert_eq!(m.train_locations.len(), 3);
        assert_eq!(m.train_locations[2], m.train_locations[1]);
        m.set_attribute(attribute::TRAIN_LOCATIONS_TOTAL_NUM, -4.0, &world(false));
        assert_eq!(m.train_locations, []);
        assert_eq!(m.train_location_entry, -1);
        m.train_locations = vec![barracks(21), barracks(30)];
        m.train_location_entry = 0;
        // ~(−1) = 0: the first goes, the entry stays at 0
        m.multiply_attribute(attribute::TRAIN_LOCATIONS_ENTRY_MOD, -1.0, &world(false));
        assert_eq!(
            (m.train_locations.len(), m.train_locations[0].time),
            (1, 30)
        );
        m.multiply_attribute(attribute::TRAIN_LOCATIONS_ENTRY_MOD, 5.0, &world(false));
        assert_eq!((m.train_locations.len(), m.train_location_entry), (2, 1));
        m.follow(
            attribute::TRAIN_TIME,
            &std::collections::BTreeMap::new(),
            None,
        );
        assert_eq!((m.trained_at.clone(), m.build_time), (vec![12, 12], 30));
    }

    /// The damage sprites: a resize zeroes new entries (a NaN count is the most, `MAXSS`); with
    /// none left the selected entry stays (an unsigned byte compare); a multiply of the selected
    /// entry removing the last leaves 0xff.
    #[test]
    fn the_damage_sprite_attributes_resize_and_select() {
        let mut m = of_type(master_type::ANIMATED);
        m.damage_graphic_entry = 2;
        m.set_attribute(attribute::DAMAGE_GRAPHICS_TOTAL_NUM, 2.0, &world(false));
        assert_eq!(m.damage_graphics, vec![DamageGraphic::default(); 2]);
        assert_eq!(m.damage_graphic_entry, 1);
        m.set_attribute(attribute::DAMAGE_GRAPHIC, 40.0, &world(false));
        m.set_attribute(attribute::DAMAGE_GRAPHIC_PERCENT, 75.0, &world(false));
        assert_eq!(m.damage_graphics[1].sprite, 40);
        assert_eq!(m.damage_graphics[1].percent, 75);
        m.set_attribute(
            attribute::DAMAGE_GRAPHICS_TOTAL_NUM,
            f32::NAN,
            &world(false),
        );
        assert_eq!(m.damage_graphics.len(), 255);
        m.set_attribute(attribute::DAMAGE_GRAPHICS_TOTAL_NUM, 0.0, &world(false));
        assert_eq!(m.damage_graphic_entry, 1);
        m.damage_graphics = vec![DamageGraphic::default()];
        m.multiply_attribute(attribute::DAMAGE_GRAPHICS_ENTRY_MOD, -3.0, &world(false));
        assert_eq!(m.damage_graphics, []);
        assert_eq!(m.damage_graphic_entry, 0xff);
        m.follow(
            attribute::DAMAGE_GRAPHICS_ENTRY_MOD,
            &std::collections::BTreeMap::new(),
            None,
        );
        assert!(!m.damage_sprites);
    }

    /// `FUN_14123d310`: a building master's garrison heal rate, foundation and garrison type are
    /// its own; its six sprites go to a copy (nothing changes); the rest falls to the combat
    /// master's setter (the regeneration from the hit points, 120).
    #[test]
    fn a_building_master_s_setters_fall_back_on_the_combat_master_s() {
        let mut m = of_type(master_type::BUILDING);
        m.building = Some(BuildingMaster::default());
        m.hit_points = 600;
        m.set_attribute(attribute::GARRISON_HEAL_RATE, 2.0, &world(false));
        m.multiply_attribute(attribute::GARRISON_HEAL_RATE, 1.5, &world(false));
        m.set_attribute(attribute::CONSTRUCTION_GRAPHIC, 9.0, &world(false));
        m.set_attribute(attribute::REGENERATION_HP_PERCENT, 0.5, &world(false));
        m.add_attribute(attribute::FOUNDATION_TERRAIN, 2.0, &world(false));
        let b = m.building.unwrap();
        assert_eq!((b.garrison_heal_rate, b.foundation_terrain), (3.0, 2));
        assert_eq!(m.regeneration, 300.0);
        assert_eq!(m, {
            let mut same = of_type(master_type::BUILDING);
            same.building = Some(b);
            same.hit_points = 600;
            same.regeneration = 300.0;
            same
        });
    }

    /// What a unit moving with a master reads follows the master's values: its radii, its
    /// speeds by its sprites' factors, its turn time and its facets.
    #[test]
    fn a_moving_master_s_derived_values_follow_its_attributes() {
        let graphic = |id, speed_multiplier, facets| crate::sprite::Graphic {
            id,
            frames: 10,
            facets,
            speed_multiplier,
            frame_duration: 0.1,
            replay_delay: 0.0,
            flags: 1,
        };
        let graphics = [
            (1, graphic(1, 0.0, 8)),
            (2, graphic(2, 1.5, 8)),
            (3, graphic(3, 0.5, 16)),
        ]
        .into_iter()
        .collect();
        let mut m = of_type(master_type::COMBAT);
        m.standing_sprite = 1;
        m.move_sprite = 2;
        m.run_sprite = -1;
        m.max_speed = 1.0;
        m.radius = [0.2, 0.2];
        m.moving = Some(MoveStats {
            fly_mode: 0,
            speed: 1.5,
            walk_speed: 1.5,
            run_speed: 1.5,
            stand_speed: 1.0,
            turn_rate: 1.0,
            turn_radius: 0.0,
            turn_radius_speed: f32::MAX,
            radius: [0.2, 0.2, 2.0],
            restriction: 7,
            restriction_mask: 1,
            turn_time: 0.0,
            move_algorithm: 0,
            min_collision: 0.0,
            fast_turn: false,
            facets: None,
            weapon_range: 0.0,
        });
        m.multiply_attribute(attribute::MOVEMENT_SPEED, 1.1, &world(false));
        m.follow(attribute::MOVEMENT_SPEED, &graphics, None);
        let s = m.moving.unwrap();
        assert_eq!(
            (s.speed, s.walk_speed, s.stand_speed),
            (1.1 * 1.5, 1.1 * 1.5, 1.1)
        );
        m.set_attribute(attribute::SIZE_Z, 1.0, &world(false));
        m.follow(attribute::SIZE_Z, &graphics, None);
        assert!(m.tall);
        assert_eq!(m.moving.unwrap().radius, [0.2, 0.2, 1.0]);
        m.set_attribute(attribute::RUNNING_GRAPHIC, 3.0, &world(false));
        m.follow(attribute::RUNNING_GRAPHIC, &graphics, None);
        let s = m.moving.unwrap();
        assert_eq!((s.speed, s.run_speed), (1.1 * 0.5, 1.1 * 0.5));
        // 8 facets and 16: no common count
        assert_eq!(s.facets, None);
        m.set_attribute(attribute::RUNNING_GRAPHIC, 2.0, &world(false));
        m.follow(attribute::RUNNING_GRAPHIC, &graphics, None);
        assert_eq!(
            m.moving.unwrap().facets,
            Some(crate::facing::SpriteFacets {
                count: 8,
                flag_1: false
            })
        );
    }

    /// The action master's (`FUN_14109fbb0` and its add and multiply) work rate, search radius
    /// and run pattern; the moving master's (`FUN_1410e1c00` ...) turn speed and trail; the
    /// interface's sounds change nothing.
    #[test]
    fn the_action_and_moving_setters_change_their_values() {
        let mut m = of_type(master_type::ACTION);
        m.set_attribute(attribute::WORK_RATE, 0.5, &world(false));
        m.add_attribute(attribute::WORK_RATE, 0.25, &world(false));
        m.multiply_attribute(attribute::WORK_RATE, 2.0, &world(false));
        m.set_attribute(attribute::SEARCH_RADIUS, 4.0, &world(false));
        m.multiply_attribute(attribute::SEARCH_RADIUS, 1.5, &world(false));
        m.set_attribute(attribute::RUN_PATTERN, 2.0, &world(false));
        m.add_attribute(attribute::RUN_PATTERN, 255.0, &world(false));
        m.set_attribute(attribute::ROTATION_SPEED, 0.5, &world(false));
        m.multiply_attribute(attribute::ROTATION_SPEED, 2.0, &world(false));
        m.set_attribute(attribute::TRAILING_UNIT, 1300.0, &world(false));
        m.add_attribute(attribute::TRAILING_UNIT, 2.0, &world(false));
        m.set_attribute(attribute::TRAIL_MODE, 3.0, &world(false));
        m.set_attribute(attribute::TRAIL_DENSITY, 1.5, &world(false));
        m.multiply_attribute(attribute::TRAIL_DENSITY, 2.0, &world(false));
        let before = m.clone();
        m.set_attribute(attribute::COMMAND_SOUND, 9.0, &world(false));
        m.add_attribute(attribute::COMMAND_SOUND, 9.0, &world(false));
        assert_eq!(m, before);
        assert_eq!((m.work_rate, m.search_radius, m.run_pattern), (1.5, 6.0, 1));
        assert_eq!(m.turn_speed, 1.0);
        assert_eq!(
            m.trail,
            Trail {
                unit: 1302,
                mode: 3,
                density: 3.0
            }
        );
    }

    /// The combat master's own values (`FUN_1412661d0` and its add and multiply): hero status,
    /// charge, conversion, regeneration (by the hit points' share for 120), volleys, the second
    /// attack sprite (an add from its id), the spawning area; the ability's interface values
    /// change nothing.
    #[test]
    fn the_combat_master_s_own_values_change() {
        let mut m = of_type(master_type::COMBAT);
        m.hit_points = 100;
        m.attack_sprite2 = 10;
        m.set_attribute(attribute::HERO_STATUS, 1.0, &world(false));
        m.add_attribute(attribute::HERO_STATUS, 4.0, &world(false));
        m.set_attribute(attribute::MAXIMUM_CHARGE, 10.0, &world(false));
        m.multiply_attribute(attribute::MAXIMUM_CHARGE, 1.5, &world(false));
        m.set_attribute(attribute::RECHARGE_RATE, 0.5, &world(false));
        m.set_attribute(attribute::CHARGE_EVENT, 2.0, &world(false));
        m.set_attribute(attribute::CHARGE_TYPE, 6.0, &world(false));
        m.set_attribute(attribute::MAX_CONVERSION_TIME_MODIFIER, 3.0, &world(false));
        m.add_attribute(attribute::MAX_CONVERSION_TIME_MODIFIER, 1.0, &world(false));
        m.set_attribute(attribute::REGENERATION_HP_PERCENT, 0.25, &world(false));
        m.add_attribute(attribute::REGENERATION_RATE, 5.0, &world(false));
        m.set_attribute(attribute::TOTAL_MISSILES, 2.0, &world(false));
        m.multiply_attribute(attribute::MAXIMUM_TOTAL_MISSILES, 3.0, &world(false));
        m.add_attribute(attribute::ATTACK_GRAPHIC_2, 2.0, &world(false));
        m.set_attribute(attribute::SPAWNING_AREA_WIDTH + 2, 0.5, &world(false));
        m.set_attribute(attribute::SECONDARY_PROJECTILE_UNIT, -1.0, &world(false));
        m.set_attribute(attribute::CHARGE_PROJECTILE_UNIT, 1500.0, &world(false));
        let before = m.clone();
        m.set_attribute(attribute::ABILITY_BUTTON_ICON_ID, 9.0, &world(false));
        assert_eq!(m, before);
        assert_eq!((m.hero_flag, m.max_charge, m.recharge_rate), (5, 15.0, 0.5));
        assert_eq!((m.charge_event, m.charge_type), (2, 6));
        assert_eq!(m.conversion, [0.0, 4.0, 0.0]);
        assert_eq!(m.regeneration, 30.0);
        assert_eq!(
            (m.volley_fire, m.max_volley, m.attack_sprite2),
            (2.0, 0, 12)
        );
        assert_eq!(m.volley_spread, [0.0, 0.0, 0.5]);
        assert_eq!((m.volley_missile, m.charge_projectile), (None, Some(1500)));
    }

    /// The missile master's add (`FUN_1410d5ab0`) adds to its own bytes and arc, then runs the
    /// RGE combat master's; a building master's add (`FUN_14123d530`) its facet and garrison
    /// type; an animated master's set (`FUN_1410a8b00`) its speed.
    #[test]
    fn the_missile_building_and_animated_setters() {
        let mut m = of_type(master_type::MISSILE);
        m.missile_stats = Some(crate::missile::MissileStats {
            projectile_type: 0,
            smart_mode: 0,
            hit_mode: 1,
            vanish_mode: 0,
            arc: 0.25,
            speed: 7.0,
            radius: [0.0; 3],
            fly_mode: 0,
        });
        m.add_attribute(attribute::PROJECTILE_SMART_MODE, 1.0, &world(false));
        m.add_attribute(attribute::PROJECTILE_HIT_MODE, 1.0, &world(false));
        m.add_attribute(attribute::PROJECTILE_ARC, 0.25, &world(false));
        m.set_attribute(attribute::MAXIMUM_RANGE, 5.0, &world(false));
        let s = m.missile_stats.unwrap();
        assert_eq!((s.smart_mode, s.hit_mode, s.arc), (1, 2, 0.5));
        assert_eq!(m.weapon_range, 5.0);
        let mut b = of_type(master_type::BUILDING);
        b.building = Some(BuildingMaster::default());
        b.add_attribute(attribute::BUILDING_ICON_OVERRIDE, 3.0, &world(false));
        b.add_attribute(attribute::GARRISON_TYPE, 2.0, &world(false));
        assert_eq!(
            (b.building_facet, b.building.unwrap().garrison_type),
            (3, 2)
        );
        let mut a = of_type(master_type::DOPPELGANGER);
        a.set_attribute(attribute::MOVEMENT_SPEED, 1.2, &world(false));
        assert_eq!(a.max_speed, 1.2);
    }

    /// The master copy (slot 8, `Master::copy_from`): every value but the id, class,
    /// availability, disabling and base id (`+0x260`).
    #[test]
    fn a_master_copy_keeps_its_id_class_and_flags() {
        let mut militia = master(74, 1, vec![task(7, -1, -1, 0.0)]);
        militia.class = 6;
        militia.available = true;
        militia.base_id = 74;
        let mut man = master(75, 2, vec![task(7, -1, -1, 0.0), task(3, 3, -1, 0.0)]);
        man.class = 7;
        man.hit_points = 45;
        man.disabled = true;
        militia.copy_from(&man);
        assert_eq!((militia.id, militia.class, militia.base_id), (74, 6, 74));
        assert!(militia.available && !militia.disabled);
        assert_eq!((militia.hit_points, militia.tasks.len()), (45, 2));
        assert_eq!(militia.task_by_group, 2);
    }

    #[test]
    fn a_villager_takes_the_first_master_of_its_swap_group_with_a_task_for_its_foundation() {
        let m = villagers();
        let site = TaskTarget {
            id: 10028,
            master: m.get(70).unwrap(),
            player: 2,
            state: 0,
            held: (-1, 0.0),
            hp: 1.0,
            max_hp: 550.0,
            attackable: true,
            pasture: -1,
            top_action: None,
        };
        let pick = |own| {
            let actor = Actor {
                id: 9181,
                master: m.get(own).unwrap(),
                player: 2,
                held: (-1, 0.0),
                attribute_95: 0.0,
                civ: 1,
            };
            m.work_master(&actor, &site, &site, -1, &Alone)
                .map(|w| w.map(|(id, t)| (id, t.action_type)))
        };
        assert_eq!(pick(83), Ok(Some((118, 101))));
        assert_eq!(pick(293), Ok(Some((212, 101))));
        assert_eq!(pick(118), Ok(Some((118, 101))));
    }
}
