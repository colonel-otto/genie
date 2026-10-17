//! Print the saved unit AI of a player's objects: its order, action and target with their last
//! values, the target's class and point, the timers, a boar's origin, the actions. For the tick
//! oracle's snapshots (`load_oracle.playback --snapshot TICK`), the game's own saves.
//!
//! cargo run --release -p genie-oracle --example unit_ais -- SAVE PLAYER ID...

use genie_rec::de;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let [_, path, player, ids @ ..] = &args[..] else {
        eprintln!("usage: unit_ais SAVE PLAYER ID...");
        std::process::exit(2);
    };
    let ids: Vec<i32> = ids
        .iter()
        .map(|i| i.parse().expect("an object id"))
        .collect();
    let data = std::fs::read(path).expect("the save");
    let chapter = de::chapters(&data).expect("chapters")[0];
    let header = chapter.header().expect("the world save");
    let player: usize = player.parse().expect("a player number");
    for object in header.players()[player].live_objects() {
        let Some(s) = object.static_part() else {
            continue;
        };
        if !ids.contains(&s.id) {
            continue;
        }
        let Some(combat) = object.combat() else {
            continue;
        };
        let Some(ai) = combat.unit_ai() else {
            continue;
        };
        println!(
            "{} master {} at ({}, {}) state {} unit-AI type {}",
            s.id, s.master_obj, s.t_x, s.t_y, s.object_state, combat.need_unit_ai
        );
        if let Some(e) = combat.tribe_elephant_unit_ai_module.as_ref() {
            println!(
                "  origin ({}, {}, {}) furious {}",
                e.origin_x, e.origin_y, e.origin_z, e.furious
            );
        }
        println!(
            "  order {} action {} target {} (class {}, point ({}, {}, {})); last order {} action {} target {} (class {})",
            ai.current_order_value,
            ai.current_action_value,
            ai.current_target_value,
            ai.current_target_type_value,
            ai.current_target_xvalue,
            ai.current_target_yvalue,
            ai.current_target_zvalue,
            ai.last_order_value,
            ai.last_action_value,
            ai.last_target_value,
            ai.last_target_type_value
        );
        println!(
            "  idle {} / {} (base {}) secondary {} look around {} / {} retarget at {} every {} stance {}",
            ai.idle_timer_value,
            ai.adjusted_idle_timeout_value,
            ai.idle_timeout_value,
            ai.secondary_timer_value,
            ai.look_around_timer_value,
            ai.look_around_timeout_value,
            ai.last_retarget_time,
            ai.randomized_retarget_timer,
            ai.state_value
        );
        let actions: Vec<_> = object
            .actions()
            .map(|a| {
                a.entries()
                    .map(|(kind, a)| (kind, a.base().map_or(0, |b| b.state)))
                    .collect()
            })
            .unwrap_or_default();
        println!(
            "  actions {actions:?} notifies {} orders {}",
            ai.number_notifies, ai.number_orders
        );
    }
}
