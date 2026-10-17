//! The task finder against the game's own: `GENIE_TASKFINDER` names runs of the game's task finder
//! (a path list; `load_oracle/probes/taskfinder.py`: each of a player's units, with each master of
//! its task swap group, on every object within 12 tiles), `GENIE_SNAPSHOTS` the directory with the
//! save of each run's tick, `GENIE_DAT` the data file (its `dropsites.json` beside it). Skips
//! without them.

mod common;

use genie_core::tasks::find_task;

/// The tick of a probe run's save, from its name (`taskfinder_<game>_<tick>[_pN].txt`).
fn tick_of(log: &std::path::Path) -> usize {
    let stem = log.file_stem().unwrap().to_str().unwrap();
    stem.split('_').nth(2).unwrap().parse().unwrap()
}

#[test]
fn the_slice_s_task_finder_finds_the_game_s_tasks() {
    let (Some(logs), Some(dir), Some(masters)) = (
        std::env::var_os("GENIE_TASKFINDER"),
        std::env::var_os("GENIE_SNAPSHOTS"),
        common::masters(),
    ) else {
        return eprintln!("GENIE_TASKFINDER / GENIE_SNAPSHOTS / GENIE_DAT are not set; skipping");
    };
    assert!(
        !masters.drop_sites.is_empty(),
        "no dropsites.json beside GENIE_DAT"
    );
    let (mut checked, mut wrong, mut stopped) = (0, Vec::new(), Vec::new());
    for log in std::env::split_paths(&logs) {
        let save = std::path::Path::new(&dir).join(format!("{}.aoe2record", tick_of(&log)));
        let world = genie_oracle::finder_world(&std::fs::read(save).unwrap(), &masters).unwrap();
        let text = std::fs::read_to_string(&log).unwrap();
        for line in text.lines() {
            // find UNIT MASTER TARGET -> ID ACTION CLASS UNIT | none
            let w: Vec<&str> = line.split_whitespace().collect();
            let (unit, master, target): (i32, i16, i32) = (
                w[1].parse().unwrap(),
                w[2].parse().unwrap(),
                w[3].parse().unwrap(),
            );
            let game = if w[5] == "none" {
                None
            } else {
                Some((
                    w[5].parse::<i16>().unwrap(),
                    w[6].parse::<i16>().unwrap(),
                    w[7].parse::<i16>().unwrap(),
                    w[8].parse::<i16>().unwrap(),
                ))
            };
            let actor = world.actor(unit).expect("the unit is in the save");
            let list = world.masters[actor.player]
                .get(master)
                .expect("the master is the player's");
            let (Some(t), Some(general)) = (world.target(target), world.target_up(target, 2))
            else {
                stopped.push(format!("{line}: a target the save does not hold"));
                continue;
            };
            match find_task(list, &actor, Some(&t), Some(&general), None, -1, &world) {
                Ok(found) => {
                    let ours = found.map(|k| (k.id, k.action_type, k.object_group, k.object_id));
                    if ours != game {
                        wrong.push(format!("{line}: slice {ours:?}"));
                    }
                    checked += 1;
                }
                Err(e) => stopped.push(format!("{line}: {e:?}")),
            }
        }
    }
    eprintln!("{checked} calls checked, {} not modelled", stopped.len());
    for s in stopped.iter().take(20) {
        eprintln!("  {s}");
    }
    assert!(
        wrong.is_empty(),
        "{} of {checked} differ: {:#?}",
        wrong.len(),
        &wrong[..wrong.len().min(20)]
    );
    assert!(3000 < checked);
}
