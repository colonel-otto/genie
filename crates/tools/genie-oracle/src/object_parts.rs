//! A saved object's lifecycle fields and its unit AI.

use crate::masters::MasterTable;
use crate::vec3;
use genie_core::lifecycle::{Class, Lifecycle, MasterData};
use genie_save::generated::world::{PlayerObjects, RgeStaticObject};

/// The object's lifecycle fields. An object with its own master takes every master value from it;
/// otherwise the death sprite and spawns come from its player's master in the save, and the decay
/// and undead flag from the table, which the save lacks.
pub(super) fn lifecycle(
    object: &genie_save::Object,
    s: &RgeStaticObject,
    objects: &PlayerObjects,
    civ: u8,
    masters: Option<&MasterTable>,
) -> Lifecycle {
    let pure_static = object
        .rge_object
        .as_deref()
        .is_some_and(|r| r.rge_static_object.is_some());
    let death_sprite = s.own_master().map_or_else(
        || {
            objects
                .master(s.master_obj)
                .and_then(|m| m.static_part())
                .map_or(-1, |m| m.death_sprite_id)
        },
        |own| own.death_sprite_num,
    );
    // the sprite after the death: the dying one, else the current one
    let after = if death_sprite >= 0 {
        death_sprite
    } else {
        s.sprite
    };
    let death_facets = masters
        .and_then(|t| t.graphics.get(&after))
        .filter(|g| pure_static && g.flags & 4 != 0 && 1 < g.facets)
        .map(|g| g.facets);
    let (dying_sprite, [death_spawn, remains], master) = s.own_master().map_or_else(
        || {
            let player = objects.master(s.master_obj).and_then(|m| m.static_part());
            (
                player.map(|m| m.death_sprite_id).filter(|&d| d >= 0),
                player.map_or([-1, -1], |m| [m.death_spawn_obj_id, m.death_spawn_obj_id2]),
                masters.and_then(|t| t.get(civ, objects.dat_master(s.master_obj))),
            )
        },
        |own| {
            (
                (own.death_sprite_num >= 0).then_some(own.death_sprite_num),
                [own.death_spawn_obj_id, own.death_spawn_obj_id2],
                Some(MasterData {
                    decay: own.attribute_rot,
                    undead: own.undead_flag != 0,
                }),
            )
        },
    );
    Lifecycle {
        class: if pure_static {
            Class::Static
        } else {
            Class::Animated
        },
        state: s.object_state,
        hp: s.hp,
        attribute_type: s.attribute_type_held,
        amount: s.attribute_amount_held,
        dying_sprite,
        death_spawn: death_spawn >= 0,
        remains,
        death_facets,
        inside: (s.inside_obj != -1).then_some(s.inside_obj),
        goto_sleep: s.goto_sleep_flag != 0,
        master_id: s.master_obj,
        master,
    }
}

pub(super) fn unit_ai(object: &genie_save::Object) -> Option<genie_core::unit_ai::UnitAi> {
    let combat = object.combat()?;
    let ai = combat.unit_ai()?;
    Some(genie_core::unit_ai::UnitAi {
        ai_type: combat.need_unit_ai as u32,
        current_order: ai.current_order_value,
        order_priority: ai.current_order_priority_value,
        current_action: ai.current_action_value,
        current_target: ai.current_target_value,
        target_type: ai.current_target_type_value,
        target_point: [
            ai.current_target_xvalue,
            ai.current_target_yvalue,
            ai.current_target_zvalue,
        ],
        desired_distance: ai.desired_target_distance_value,
        defend_target: ai.defend_target_value,
        last_order: ai.last_order_value,
        last_action: ai.last_action_value,
        last_target: ai.last_target_value,
        last_target_type: ai.last_target_type_value,
        idle_timer: ai.idle_timer_value,
        adjusted_idle_timeout: ai.adjusted_idle_timeout_value,
        idle_timeout: ai.idle_timeout_value,
        secondary_timer: ai.secondary_timer_value,
        look_around_timer: ai.look_around_timer_value,
        look_around_timeout: ai.look_around_timeout_value,
        last_retarget_time: ai.last_retarget_time,
        retarget_timer: ai.randomized_retarget_timer,
        stance: ai.state_value,
        formation_type: ai.formation_type,
        state_position: [ai.state_position_xvalue, ai.state_position_yvalue],
        retarget_disabled: ai.disable_retargeting,
        alert_mode: ai.alert_mode_value != 0,
        time_since_enemy: ai.time_since_enemy_sighting,
        // the saved patrol (its path's points in the game's layout, length and flags)
        patrol_path: ai
            .patrol_path
            .as_deref()
            .map(|p| genie_core::unit_ai::PatrolPath {
                path: genie_core::tile_path::Path {
                    points: p.m_waypoints.iter().map(vec3).collect(),
                    length: p.path_length_value,
                },
                flags: p.flags_m_bits,
            }),
        patrol_waypoint: ai.patrol_current_waypoint,
        stop_after_kill: ai.stop_after_target_killed_value != 0,
        home: combat
            .tribe_elephant_unit_ai_module
            .as_deref()
            .map(|e| genie_core::unit_ai::Home {
                origin: [e.origin_x, e.origin_y, e.origin_z],
                furious: e.furious != 0,
                go_home: false,
            }),
        hunted: None,
        last_update_time: ai.last_update_time_value,
        attackers: ai.temp_int.clone(),
        // the order history in the save's order, the newest first (the load appends each at the
        // tail, `0x140c4efc0`)
        order_history: ai
            .oh
            .iter()
            .map(|e| genie_core::unit_ai::OrderRecord {
                order: e.new_oh_order,
                action: e.new_oh_action,
                time: e.new_oh_time,
                position: vec3(&e.new_oh_position),
                target: e.new_oh_target_id,
                category: e.new_oh_target_attack_category,
                target_position: vec3(&e.new_oh_target_position),
            })
            .collect(),
        retries: ai
            .retarget_entries
            .iter()
            .map(|e| (e.target_id, e.timeout))
            .collect(),
        best_this_update: ai.best_unit_to_attack_this_update,
        orders: ai
            .temp_order
            .iter()
            .map(genie_core::unit_ai::QueuedOrder::from_bytes)
            .collect(),
        building_orders: combat
            .tribe_tower_unit_ai_module
            .as_deref()
            .map(|t| &t.saved_order)
            .or_else(|| {
                combat
                    .tribe_building_unit_ai_module
                    .as_deref()
                    .map(|b| &b.saved_order)
            })
            .map(|orders| {
                orders
                    .iter()
                    .map(genie_core::unit_ai::QueuedOrder::from_bytes)
                    .collect()
            })
            .unwrap_or_default(),
        notifies: ai
            .temp_notify
            .iter()
            .map(genie_core::unit_ai::Notify::from_bytes)
            .collect(),
        // the load's factory capacity, doubled for what it reads
        notify_capacity: genie_core::unit_ai::notify_capacity(
            combat.need_unit_ai as u32,
            ai.temp_notify.len(),
        ),
        processing: false,
        buffered: Vec::new(),
        commanded: false,
    })
}
