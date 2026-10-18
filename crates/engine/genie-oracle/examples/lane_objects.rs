//! Print a player's live objects from a recording's header save: id, master, object state, hit
//! points, position, the top action (type and state) and the unit AI's retarget timer. For the
//! tick oracle's snapshots (`load_oracle.playback --snapshot TICK`), the game's own saves.
//!
//! cargo run --release -p genie-oracle --example `lane_objects` -- REC PLAYER

use genie_rec::de;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (path, player, verbose) = match &args[..] {
        [_, path, player] => (path, player, None),
        [_, path, player, id] => (path, player, Some(id.parse::<i32>().expect("an object id"))),
        _ => {
            eprintln!(
                "usage: lane_objects REC PLAYER [OBJECT_ID]: an id prints that object's actions"
            );
            std::process::exit(2);
        }
    };
    let data = std::fs::read(path).expect("the recording");
    let chapter = de::chapters(&data).expect("chapters")[0];
    let header = chapter.header().expect("the world save");
    let player: usize = player.parse().expect("a player number");
    let section = &header.players()[player];
    println!("id\tmaster\tstate\thp\tx\ty\tz\taction\taction_state\tretarget");
    for object in section.live_objects() {
        let Some(s) = object.static_part() else {
            continue;
        };
        if verbose == Some(s.id) {
            println!("{:#?}", action_object(object).map(|a| &a.actions));
        }
        let top = action_object(object).and_then(|a| a.actions.action_type_list.first());
        let (action, state) = top.map_or((-1, String::new()), |e| {
            let debug = format!("{:?}", e.action);
            let state = debug
                .split("state: ")
                .nth(1)
                .and_then(|r| r.split([',', ' ']).next())
                .unwrap_or("")
                .to_string();
            (e.action_type, state)
        });
        let retarget = object
            .combat()
            .and_then(|c| c.unit_ai())
            .map_or(String::new(), |ai| ai.randomized_retarget_timer.to_string());
        println!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            s.id, s.master_obj, s.object_state, s.hp, s.t_x, s.t_y, s.t_z, action, state, retarget
        );
    }
}

fn action_object(o: &genie_save::Object) -> Option<&genie_save::generated::world::RgeActionObject> {
    if let Some(c) = o.combat() {
        return Some(&c.rge_combat_object.rge_action_object);
    }
    let r = o.rge_object.as_ref()?;
    r.rge_action_object
        .as_ref()
        .or(r.rge_combat_object.as_ref().map(|c| &c.rge_action_object))
}
