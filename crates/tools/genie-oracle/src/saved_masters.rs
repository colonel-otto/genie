//! A player's saved masters as the engine reads them: the facets, the action part and
//! tasks, the work command's masters, the movement and the placement.

use crate::masters::MasterTable;
use crate::{animated_master, combat_master, moving_master};
use genie_core::GenieError;
use genie_save::generated::world::{Master, PlayerObjects, RgeStaticObject};

/// The facets every sprite a live object of a saved master shows ([`genie_core::objects::facing::SpriteFacets`]):
/// its standing, second standing, walking, running and attack sprites and its tasks' four, those
/// the data file has, all of one count and bit 1 of the sequence flags; None when they differ or
/// it has none. The dying sprite shows once the object is dead, where nothing turns it.
pub(super) fn master_facets(
    m: &Master,
    s: &genie_save::generated::world::RgeMasterStaticObject,
    table: &MasterTable,
) -> Option<genie_core::objects::facing::SpriteFacets> {
    let mut sprites = vec![s.idle_sprite_id, s.idle2_sprite_id];
    if let Some(mv) = moving_master(m) {
        sprites.extend([mv.move_sprite_id, mv.run_sprite_id]);
    }
    if let Some(c) = combat_master(m) {
        sprites.push(c.fight_sprite_id);
    }
    if let Some(tasks) = action_part(m).and_then(|a| a.tasks.as_deref()) {
        for e in &tasks.list {
            let t = &e.task;
            sprites.extend([t.short_val, t.short_val_2, t.short_val_3, t.short_val_4]);
        }
    }
    let mut seen = sprites
        .into_iter()
        .filter(|&id| 0 <= id)
        .filter_map(|id| table.graphics.get(&id))
        .map(|g| genie_core::objects::facing::SpriteFacets {
            count: g.facets,
            flag_1: g.flags & 2 != 0,
        });
    let first = seen.next()?;
    seen.all(|f| f == first).then_some(first)
}

/// A master's action part, whatever class it is: what holds its work rate, search radius and
/// task list.
pub(super) fn action_part(
    m: &Master,
) -> Option<&genie_save::generated::world::RgeMasterActionObject> {
    None.or_else(|| {
        m.tribe_master_combat_object
            .as_deref()
            .map(|c| &c.rge_master_combat_object.rge_master_action_object)
    })
    .or_else(|| {
        m.tribe_master_building_object.as_deref().map(|b| {
            &b.tribe_master_combat_object
                .rge_master_combat_object
                .rge_master_action_object
        })
    })
    .or_else(|| {
        m.rge_master.as_deref().and_then(|r| {
            r.rge_master_action_object.as_deref().or_else(|| {
                r.rge_master_combat_object
                    .as_deref()
                    .map(|c| &c.rge_master_action_object)
            })
        })
    })
}

/// The task at `index` in a master's task list (its saved list, else the `.dat`'s): the task a
/// saved action's base `short_val` names (the base load `FUN_140a7a2f0` resolves it to `+0x48`
/// at `0x140a7b5e8`).
pub(super) fn task_at(
    objects: &PlayerObjects,
    civ: u8,
    masters: Option<&MasterTable>,
    master: i16,
    index: i16,
) -> Result<genie_core::objects::masters::Task, GenieError> {
    let i = usize::try_from(index)
        .map_err(|_| GenieError::unsupported("a saved action without its task"))?;
    let saved = objects
        .master(master)
        .and_then(action_part)
        .and_then(|a| a.tasks.as_deref());
    if let Some(tasks) = saved {
        return tasks
            .list
            .get(i)
            .map(|e| core_task(&e.task))
            .ok_or(GenieError::unsupported(
                "a saved action's task beyond its master's list",
            ));
    }
    masters
        .and_then(|t| t.work(civ, objects.dat_master(master)))
        .and_then(|w| w.tasks.get(i))
        .copied()
        .ok_or(GenieError::unsupported(
            "a saved action's task the data file does not have",
        ))
}

pub(super) const fn core_task(
    t: &genie_save::generated::world::Task,
) -> genie_core::objects::masters::Task {
    genie_core::objects::masters::Task {
        id: t.id,
        action_type: t.action_type,
        object_group: t.object_group,
        object_id: t.object_id,
        combat_level: t.combat_level,
        combat_level_flag: t.combat_level_flag,
        terrain: t.terrain_id,
        owner_type: t.owner_type,
        holding_attr: t.holding_attr,
        attribute_type: t.attribute_type,
        attribute_type2: t.attribute_type2,
        attribute_type3: t.attribute_type3,
        attribute_type4: t.attribute_type4,
        wait_time: t.search_wait_time,
        work_val1: t.work_val1,
        work_val2: t.work_val2,
        work_range: t.work_range,
        work_flag: t.work_flag,
        enablement: t.enablement_resource_id,
        sprites: [t.short_val, t.short_val_2, t.short_val_3, t.short_val_4],
        sound_events: [
            t.gather_sound_event_event_id.cast_signed(),
            t.deposit_sound_event_event_id.cast_signed(),
        ],
    }
}

/// A player's masters as the work command reads them: each master the save holds, with its saved
/// class, radii, cost and (where saved) tasks, and the `.dat`'s task swap group and type, which
/// the save does not carry (the game copies them from the `.dat` master).
pub(super) fn work_masters(
    objects: &PlayerObjects,
    civ: u8,
    table: &MasterTable,
) -> genie_core::objects::masters::Masters {
    let task = core_task;
    let saved = objects.masters.iter().flatten().map(|m| &m.master);
    genie_core::objects::masters::Masters::new(saved.filter_map(|m| {
        let s = m.static_part()?;
        // the `.dat` values of the master it copies (an upgraded one's), under its own id
        let mut work = table.work(civ, s.copy_id())?.clone();
        work.id = s.id();
        work.class = s.read_temp_5;
        work.radius = [s.radius_x, s.radius_y];
        work.tall = 0.0 < s.radius_z;
        work.height = s.radius_z;
        work.obstruction_type = s.obstruction_type;
        work.buildable_on = s.can_be_built_on;
        work.obstruction_class = s.obstruction_class;
        work.object_flags = s.object_flags;
        work.hit_points = s.read_temp_6;
        work.available = s.available != 0;
        work.disabled = s.disabled != 0;
        work.line_of_sight = s.los;
        work.capacity = s.attribute_max_amount;
        work.blast_defense = s.area_effect_object_level;
        if let Some(a) = action_part(m) {
            work.work_rate = a.work_rate;
            work.search_radius = a.search_radius;
            if let Some(tasks) = a.tasks.as_deref() {
                work.tasks = tasks.list.iter().map(|e| task(&e.task)).collect();
            }
        }
        let combat = m.tribe_master_combat_object.as_deref().or_else(|| {
            m.tribe_master_building_object
                .as_deref()
                .map(|b| &b.tribe_master_combat_object)
        });
        if let Some(c) = combat {
            work.cost = build_cost(&c.build_inventory);
            work.build_time = c.train_location.first().map_or(0, |t| t.train_time);
            work.charge_type = c.charge_type;
            work.formation_category = c.temp_formation_category;
            work.hero_flag = c.hero_flag;
            work.volley_fire = c.volley_fire_amount;
            work.max_volley = c.max_attacks_in_volley;
            work.volley_spread = [
                c.volley_x_spread,
                c.volley_y_spread,
                c.volley_start_spread_adjustment,
            ];
            let id = |id: i32| i16::try_from(id).ok().filter(|&id| 0 <= id);
            work.volley_missile = id(c.volley_missile_id);
            work.charge_projectile = id(c.charge_projectile_unit);
            work.invulnerability = c.invulnerability_level;
            work.attack_priority = c.attack_priority;
            work.special_attack = c.special_attack_flag;
            work.max_charge = c.max_charge;
            work.charge_target = c.charge_target;
        }
        // the combat part a missile master has too (`RGE_Master_Missile_Object`'s base)
        let rge_combat = combat.map(|c| &c.rge_master_combat_object).or_else(|| {
            m.rge_master
                .as_deref()
                .and_then(|r| r.rge_master_missile_object.as_deref())
                .map(|mm| &mm.rge_master_combat_object)
        });
        if let Some(rc) = rge_combat {
            work.weapon_range = rc.weapon_range;
            work.min_range = rc.minimum_weapon_range;
            work.missile = rc.missile_id;
            work.weapon_offset = [rc.weapon_offset_x, rc.weapon_offset_y, rc.weapon_offset_z];
            work.missed_spread = rc.missed_missile_spread;
            work.attack_sprite = rc.fight_sprite_id;
            work.fire_frame = rc.fire_missile_at_frame;
            work.interrupt_frame = rc.interrupt_frame;
            work.hit_chance = rc.base_hit_chance;
            work.reload = rc.speed_of_attack;
            work.weapons = rc
                .weapon
                .iter()
                .map(|&w| ((w & 0xffff) as i16, (w >> 16) as i16))
                .collect();
            work.base_armor = rc.base_armor;
            work.armors = rc
                .armor
                .iter()
                .map(|&a| ((a & 0xffff) as i16, (a >> 16) as i16))
                .collect();
            work.resistance = rc.bonus_damage_resistance;
            work.combat_ability = rc.combat_ability;
            work.reflection = rc.damage_reflection;
            work.friendly_fire = rc.friendly_fire_damage;
            work.area_effect = rc.area_effect_range;
            work.blast_level = rc.area_effect_level;
            work.area_damage = rc.area_damage;
            work.garrison_firepower = rc.garrison_firepower;
        }
        work.storage = s
            .attribute
            .iter()
            .map(|e| {
                (
                    e.attribute_type_held,
                    e.attribute_amount_held,
                    e.attribute_flag,
                )
            })
            .collect();
        // the save holds the foundation terrain of its own building masters, its garrison's
        // kinds (`garrison_units`, read into `+0x9bc`: a civilisation's bonus, as the Gurjaras'
        // herdables in mills, changes them at the start) and rates, and its facet
        if let Some(saved) = m.tribe_master_building_object.as_deref() {
            work.building_facet = saved.building_facet.cast_signed();
        }
        if let (Some(b), Some(saved)) = (
            work.building.as_mut(),
            m.tribe_master_building_object.as_deref(),
        ) {
            b.garrison_type = saved.garrison_units;
            b.garrison_heal_rate = saved.garrison_heal_rate;
            b.garrison_repair_rate = saved.garrison_repair_rate;
            b.foundation_terrain = saved.on_build_make_tile;
            // `+0x960`, the third of its sprites (`tempId`)
            b.destruction_sprite = saved.temp_id_3.cast_signed();
            // `+0x950`, the first (`tempId`)
            b.construction_sprite = saved.temp_id.cast_signed();
        }
        // a missile master's flight: the save's three bytes and arc, the data file's projectile
        // type
        if let Some(mm) = m
            .rge_master
            .as_deref()
            .and_then(|r| r.rge_master_missile_object.as_deref())
        {
            work.missile_stats = Some(genie_core::combat::missile::MissileStats {
                projectile_type: table.projectile_type(civ, s.copy_id()).unwrap_or(0),
                smart_mode: mm.targetting_type,
                hit_mode: mm.missile_hit_info,
                vanish_mode: mm.missile_die_info,
                arc: mm.ballistics_ratio,
                speed: animated_master(m).map_or(0.0, |a| a.max_speed),
                radius: [s.radius_x, s.radius_y, s.radius_z],
                fly_mode: table.fly_mode(civ, s.copy_id()).unwrap_or(0),
            });
        }
        work.damage_sprites = !s.damage_sprite.is_empty();
        // what the masters' attribute setters change: the top and turn speeds (+0x740,
        // +0x758), the damage sprites (+0x638) and the train locations (+0x898)
        work.max_speed = animated_master(m).map_or(0.0, |a| a.max_speed);
        work.turn_speed = moving_master(m).map_or(0.0, |mv| mv.turn_speed);
        work.damage_graphics = s
            .damage_sprite
            .iter()
            .map(|d| genie_core::objects::masters::DamageGraphic {
                sprite: d.temp_id,
                percent: d.sprite_info_damage_percent,
                apply_mode: d.sprite_info_flag,
            })
            .collect();
        // a master without a combat part has no train locations, whatever its copy's are
        work.train_locations = combat.map_or_else(Vec::new, |c| {
            c.train_location
                .iter()
                .map(|t| genie_core::objects::masters::TrainLocation {
                    location: t.train_location_id,
                    time: t.train_time,
                    button: t.button_location_id,
                    hotkey: t.hotkey_id,
                })
                .collect()
        });
        work.trained_at = work.train_locations.iter().map(|t| t.location).collect();
        work.fly_mode = table.fly_mode(civ, s.copy_id()).unwrap_or(0);
        work.standing_sprite = s.idle_sprite_id;
        work.standing_sprite_2 = (0 <= s.idle2_sprite_id).then_some(s.idle2_sprite_id);
        (work.move_sprite, work.run_sprite) =
            moving_master(m).map_or((-1, -1), |mv| (mv.move_sprite_id, mv.run_sprite_id));
        work.death_sprite = s.death_sprite_id;
        work.garrison = s.obj_max;
        work.death_spawns = [s.death_spawn_obj_id, s.death_spawn_obj_id2];
        work.terrain_restriction = s.terrain_restriction_mask_id;
        work.life = table.get(civ, s.copy_id());
        work.moving = moving_stats(m, s, civ, table);
        Some(work)
    }))
}

/// What a unit of a saved master moves with ([`genie_core::movement::MoveStats`]): the master's
/// top speed times its walking sprite's speed factor when above 0 (the run sprite if it has one,
/// else the move sprite: the `MoveTo`'s, slots 2000 and 0x7c8), the `.dat`'s turning, the radii and
/// the terrain restriction with its mask.
pub(super) fn moving_stats(
    m: &Master,
    s: &genie_save::generated::world::RgeMasterStaticObject,
    civ: u8,
    table: &MasterTable,
) -> Option<genie_core::movement::MoveStats> {
    let moving = moving_master(m)?;
    let sprite = if moving.run_sprite_id >= 0 {
        moving.run_sprite_id
    } else {
        moving.move_sprite_id
    };
    let base = moving.rge_master_animated_object.max_speed;
    let factor = table.sprite_speed(sprite).unwrap_or(0.0);
    let turning = table.turning(civ, s.copy_id())?;
    let restriction = usize::try_from(s.terrain_restriction_mask_id).ok()?;
    let facets = table.graphics.get(&moving.move_sprite_id).map(|g| g.facets);
    Some(genie_core::movement::MoveStats {
        fly_mode: table.fly_mode(civ, s.copy_id()).unwrap_or(0),
        speed: if 0.0 < factor { base * factor } else { base },
        turn_rate: genie_core::movement::turn_rate(moving.turn_speed, facets),
        turn_radius: 0.0,
        turn_radius_speed: f32::MAX,
        radius: [s.radius_x, s.radius_y, s.radius_z],
        restriction,
        restriction_mask: *table.terrain.masks().get(restriction)?,
        turn_time: moving.turn_speed,
        move_algorithm: turning.move_algorithm,
        min_collision: turning.min_collision_size_multiplier,
        fast_turn: s.object_flags & 2 != 0,
        walk_speed: {
            let f = table.sprite_speed(moving.move_sprite_id).unwrap_or(0.0);
            if 0.0 < f { base * f } else { base }
        },
        run_speed: {
            let id = if 0 <= moving.run_sprite_id {
                moving.run_sprite_id
            } else {
                moving.move_sprite_id
            };
            let f = table.sprite_speed(id).unwrap_or(0.0);
            if 0.0 < f { base * f } else { base }
        },
        stand_speed: {
            let f = table.sprite_speed(s.idle_sprite_id).unwrap_or(0.0);
            if 0.0 < f { base * f } else { base }
        },
        facets: master_facets(m, s, table),
        weapon_range: combat_master(m).map_or(0.0, |c| c.weapon_range),
    })
}

/// A combat master's `build_inventory` entries `(resource, amount, flag)`, three shorts each; the
/// flag set means paid. Entries of resource -1 are empty.
pub(super) fn build_cost(inventory: &[[u8; 6]]) -> Vec<(i16, i16, bool)> {
    build_entries(inventory)
        .into_iter()
        .filter(|&(kind, ..)| kind >= 0)
        .collect()
}

/// A combat master's `build_inventory` entries `(resource, amount, paid)`, the empty ones
/// (resource −1) kept in their places.
pub(super) fn build_entries(inventory: &[[u8; 6]]) -> Vec<(i16, i16, bool)> {
    inventory
        .iter()
        .map(|e| {
            let kind = i16::from_le_bytes([e[0], e[1]]);
            let amount = i16::from_le_bytes([e[2], e[3]]);
            let flag = i16::from_le_bytes([e[4], e[5]]);
            (kind, amount, flag != 0)
        })
        .collect()
}

/// What movement reads of a saved object's master: radii, class and obstruction kind, from the
/// object's own master or its player's.
/// How fast an object may move, for the slice's bound on a frozen object's place: its own or its
/// player's master's speed, 0 for an object that does not move.
pub(super) fn moving_speed(
    o: &genie_save::Object,
    s: &RgeStaticObject,
    objects: &PlayerObjects,
) -> f32 {
    if o.moving_part().is_none() {
        return 0.0;
    }
    if s.own_master().is_some() {
        return s
            .own_animated_master()
            .map_or(f32::INFINITY, |a| a.max_speed);
    }
    objects
        .master(s.master_obj)
        .and_then(animated_master)
        .map_or(f32::INFINITY, |a| a.max_speed)
}

/// What movement and sight read of an object's master (its own or its player's).
pub(super) struct Placement {
    pub(super) radius: [f32; 3],
    pub(super) class: i16,
    pub(super) obstruction_type: u8,
    pub(super) obstruction_class: u8,
    pub(super) buildable_on: u8,
    pub(super) los: f32,
}

pub(super) fn placement(s: &RgeStaticObject, objects: &PlayerObjects) -> Option<Placement> {
    if let Some(own) = s.own_master() {
        // an own master's second readTemp is its class (its first the id, its third the hit
        // points: as the shared masters' of the same id read)
        return Some(Placement {
            radius: [own.radius_x, own.radius_y, own.radius_z],
            class: own.read_temp_2,
            obstruction_type: own.obstruction_type,
            obstruction_class: own.obstruction_class,
            buildable_on: own.can_be_built_on,
            los: own.los,
        });
    }
    let m = objects.master(s.master_obj)?.static_part()?;
    Some(Placement {
        radius: [m.radius_x, m.radius_y, m.radius_z],
        class: m.read_temp_5,
        obstruction_type: m.obstruction_type,
        obstruction_class: m.obstruction_class,
        buildable_on: m.can_be_built_on,
        los: m.los,
    })
}

#[cfg(test)]
mod tests;
