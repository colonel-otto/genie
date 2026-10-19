//! Print the saved movement state of a player's objects: position, velocity data, orientation,
//! path data with the waypoints of each path (from the world's path table), speed fields. For the
//! tick oracle's snapshots (`load_oracle.playback --snapshot TICK`), the game's own saves.
//!
//! cargo run --release -p genie-oracle --example `moving_objects` -- SAVE PLAYER ID...

use genie_rec::de;

fn f3(b: &[u8; 12]) -> [f32; 3] {
    let f = |i: usize| f32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
    [f(0), f(4), f(8)]
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let [_, path, player, ids @ ..] = &args[..] else {
        eprintln!("usage: moving_objects SAVE PLAYER ID...");
        std::process::exit(2);
    };
    let ids: Vec<i32> = ids
        .iter()
        .map(|i| i.parse().expect("an object id"))
        .collect();
    let data = std::fs::read(path).expect("the save");
    let chapter = de::chapters(&data).expect("chapters")[0];
    let header = chapter.header().expect("the world save");
    let paths = &header.world.base_world.path;
    let player: usize = player.parse().expect("a player number");
    for object in header.players()[player].live_objects() {
        let (Some(s), Some(m)) = (object.static_part(), object.moving_part()) else {
            continue;
        };
        if !ids.contains(&s.id) {
            continue;
        }
        println!(
            "{} master {} at ({}, {}, {})",
            s.id, s.master_obj, s.t_x, s.t_y, s.t_z
        );
        println!(
            "  velocity ({}, {}, {}) angle {} turn_towards {} waiting {} on_ground {}",
            m.velocity_x,
            m.velocity_y,
            m.velocity_z,
            m.angle,
            m.turn_towards_time,
            m.waiting_to_move_value,
            m.on_ground
        );
        println!(
            "  movement {:?} {:?}",
            m.velocity.as_ref().map(f3),
            m.acceleration.as_ref().map(f3)
        );
        println!(
            "  position {:?} forward {:?} right {:?} last_move {}",
            f3(&m.position_value),
            f3(&m.orientation_forward_value),
            f3(&m.orientation_right_value),
            m.last_move_time_value
        );
        println!(
            "  speedup building {} value {}",
            m.speedup_building_id, m.speedup_value
        );
        let datas = m.temp_path_data.iter().chain(m.future_path_data.as_deref());
        for d in datas {
            println!("  path data {d:?}");
            for p in paths.iter().filter(|p| p.idvalue == d.path_id) {
                let points: Vec<[f32; 3]> = p.m_waypoints.iter().map(f3).collect();
                println!(
                    "    path {} points {points:?} length {} created {} flags {:#x} refs {}",
                    p.idvalue,
                    p.path_length_value,
                    p.creation_time_value,
                    p.flags_m_bits,
                    p.reference_count
                );
            }
        }
    }
}
