//! What the engine reads of a player's masters: the tasks a unit can do and the masters a unit
//! changes into to do them. The values are game data: the player's masters in the save, with the
//! `.dat`'s values for what the save does not carry.

use crate::movement::MoveStats;
use crate::tasks::{Actor, Diplomacy, TaskTarget, find_task};
use crate::unit_ai::Unsupported;

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
    pub fn default_start(&self) -> (bool, Option<Unsupported>) {
        match self.default_task() {
            Some(t) if t.action_type == crate::make::MAKE => (true, None),
            Some(t) if ACTION_FACTORY_TYPES.contains(&t.action_type) => (
                false,
                Some(Unsupported("a new object's default task other than a Make")),
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
    ) -> Result<Option<(i16, &'m Task)>, Unsupported> {
        let own = actor.master;
        let own_list = self
            .get(own.id)
            .ok_or(Unsupported("a unit of a master the player does not have"))?;
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
