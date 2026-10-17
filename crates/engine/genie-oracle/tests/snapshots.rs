//! genie-core started from the game's own save at a tick, against the game's save some ticks
//! later, object by object in full f32 (FD-008's loop). `GENIE_SNAPSHOTS` names a directory of the
//! tick oracle's saves of MarvinBlue/426048731 (`<tick>.aoe2record`: `load_oracle.playback
//! --snapshot TICK`, the state after that many ticks) with the game's shape mesh at some of those
//! ticks (`<tick>.mesh`: `load_oracle/probes/mesh.py`, which the save does not hold),
//! `GENIE_TICK_REC` the recording, `GENIE_DAT` the build's `.dat`. Skips without them.

mod common;

use std::path::PathBuf;

fn inputs() -> Option<(PathBuf, Vec<u8>, genie_oracle::MasterTable)> {
    let dir = PathBuf::from(std::env::var_os("GENIE_SNAPSHOTS")?);
    let rec = std::fs::read(std::env::var_os("GENIE_TICK_REC")?).unwrap();
    Some((dir, rec, common::masters()?))
}

fn save(dir: &std::path::Path, tick: usize) -> Vec<u8> {
    std::fs::read(dir.join(format!("{tick}.aoe2record"))).unwrap()
}

/// The game's mesh at a tick, where the directory has it.
fn mesh(dir: &std::path::Path, tick: usize) -> Option<String> {
    std::fs::read_to_string(dir.join(format!("{tick}.mesh"))).ok()
}

/// The slice from the save after `start`, run `ticks` ticks, against the save after `start + ticks`.
fn differences(start: usize, ticks: usize, player: u32) -> Option<Vec<String>> {
    let (dir, rec, masters) = inputs()?;
    let run = genie_oracle::objects_from_save(
        &save(&dir, start),
        &rec,
        start,
        ticks,
        player,
        Some(&masters),
        mesh(&dir, start).as_deref(),
    )
    .unwrap();
    let game = genie_oracle::saved_objects(&save(&dir, start + ticks), player).unwrap();
    let mut out = genie_oracle::object_differences(&run.objects, &game);
    if let Some(u) = run.unsupported {
        out.push(format!("unsupported: {u:?}"));
        if let Some(id) = run.unsupported_id {
            eprintln!("the unmodelled update is object {id}'s");
        }
    }
    let others = run
        .stopped
        .iter()
        .enumerate()
        .filter_map(|(p, stop)| stop.filter(|_| p != player as usize).map(|u| (p, u)));
    for (p, u) in others {
        eprintln!("player {p} stopped: {u:?}");
    }
    Some(out)
}

#[test]
fn a_save_read_as_the_slice_s_start_is_the_save() {
    let Some((dir, _, masters)) = inputs() else {
        return eprintln!("GENIE_SNAPSHOTS / GENIE_TICK_REC / GENIE_DAT are not set; skipping");
    };
    let rec = std::fs::read(std::env::var_os("GENIE_TICK_REC").unwrap()).unwrap();
    let start = save(&dir, 679);
    let run =
        genie_oracle::objects_from_save(&start, &rec, 679, 0, 2, Some(&masters), None).unwrap();
    let game = genie_oracle::saved_objects(&start, 2).unwrap();
    assert_eq!(
        genie_oracle::object_differences(&run.objects, &game),
        Vec::<String>::new()
    );
    // the builders' Build actions come from the save: 9175 works next to the house, two walk
    let action = |id| run.objects.iter().find(|o| o.id == id).unwrap().action;
    assert_eq!(
        (action(9175), action(9177), action(9181)),
        (Some((101, 6)), Some((101, 4)), Some((101, 4)))
    );
}

#[test]
fn the_build_tick_from_the_save_before_it_matches_the_game_object_by_object() {
    // update 680: the far builders' MoveTos ask for their paths to the foundation; the motion
    // step plans them in the same update (9177 with the short pather, 9181 with the tile
    // pathfinder, then the short pather on its first stretch), so every object, its path data
    // included, is the game's
    let Some(found) = differences(678, 1, 2) else {
        return eprintln!("GENIE_SNAPSHOTS / GENIE_TICK_REC / GENIE_DAT are not set; skipping");
    };
    assert_eq!(found, Vec::<String>::new());
}

#[test]
fn the_walk_tick_from_the_save_before_it_matches_the_game_object_by_object() {
    // update 681: 9177 and 9181 take their first steps, 9175 faces its foundation, finds itself
    // on the footprint and turns to walk off it (Build state 223)
    let Some(found) = differences(679, 1, 2) else {
        return eprintln!("GENIE_SNAPSHOTS / GENIE_TICK_REC / GENIE_DAT are not set; skipping");
    };
    assert_eq!(found, Vec::<String>::new());
}

#[test]
fn the_walk_off_the_footprint_from_a_save_matches_the_game_object_by_object() {
    // updates 682 to 725: 9175's MoveTo off the footprint plans its path (the tile pathfinder,
    // then the short pather) and it walks it; in 724 its step reaches the waypoint, in 725 it
    // stands on it, its levels arrive and it stops; 9177 and 9181 walk on to the foundation
    let Some(found) = differences(680, 44, 2) else {
        return eprintln!("GENIE_SNAPSHOTS / GENIE_TICK_REC / GENIE_DAT are not set; skipping");
    };
    assert_eq!(found, Vec::<String>::new());
}

/// Not pinned: what the slice gets wrong some ticks after a save, for the tick FD-008 works on
/// (`GENIE_SNAPSHOT_START`, default 679; `GENIE_SNAPSHOT_TICKS`, default 1).
#[test]
fn the_slice_one_tick_after_a_save_report() {
    let start = std::env::var("GENIE_SNAPSHOT_START").map_or(679, |s| s.parse().unwrap());
    let ticks = std::env::var("GENIE_SNAPSHOT_TICKS").map_or(1, |s| s.parse().unwrap());
    let Some(found) = differences(start, ticks, 2) else {
        return eprintln!("GENIE_SNAPSHOTS / GENIE_TICK_REC / GENIE_DAT are not set; skipping");
    };
    eprintln!(
        "tick {} from the save after {start}: {found:#?}",
        start + ticks
    );
}
