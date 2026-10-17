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
    let after = save(&dir, start + ticks);
    let game = genie_oracle::saved_objects(&after, player).unwrap();
    let mut out = genie_oracle::object_differences(&run.objects, &game);
    // the player's sight: its visible map and its bits of the unified map
    let seen = genie_oracle::saved_visibility(&after).unwrap();
    let sight = run.visibility.as_ref().expect("the slice holds the sight");
    out.extend(genie_oracle::visibility_differences(
        sight,
        &seen,
        player as usize,
    ));
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
fn the_construction_from_a_save_matches_the_game_object_by_object() {
    // updates 682 to 741: 9175 walks off the footprint and back (Build 223 -> 4); 9177 reaches
    // the foundation (729), joins its work (730: state 7, a builder) and starts it with its first
    // work (731: the record, the mesh shape and the planners' tiles, a sliver of progress), then
    // works on: the house's hit points and progress are the game's
    let Some(found) = differences(680, 60, 2) else {
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

#[test]
fn the_order_to_a_tree_from_a_save_matches_the_game_object_by_object() {
    // updates 682 to 1041: in 748 player 2's INTERACT sends builder 9181 to the tree 9365: the work
    // command finds the gather task of master 218 (VFLUM), the master changes (the Build action
    // and its MoveTo go), the Hunt wakes the tree and attacks it first (Hunt state 12, Attack
    // state 4 with a MoveTo, built twice), and 9181 walks to the tree from the same update on; in
    // 797 the next order sends it to the tree 9357: the hunt keeps its task, takes the new target
    // (the old one is flagged to sleep) and starts over; in 1008 it reaches the tree: the
    // Attack turns ready (state 6) and swings (7) with its sprite from frame 0; through 1040
    let Some(found) = differences(680, 360, 2) else {
        return eprintln!("GENIE_SNAPSHOTS / GENIE_TICK_REC / GENIE_DAT are not set; skipping");
    };
    assert_eq!(found, Vec::<String>::new());
}

#[test]
fn the_scout_s_move_from_a_save_matches_the_game_object_by_object() {
    // updates 682 to 1110: in 1044 player 2's MOVE sends the scout 9185 west: it takes the move
    // itself (a MoveTo to the point), its unit AI the move order and action with the point, and
    // it plans its path past Gaia's boar 9732, whose unit AI (its idle check at 1039: a draw of
    // its leash, a rand15 that is not a multiple of 7) keeps it at its origin; on each of player
    // 2's turns the scout's AI, busy, keeps its state position, redraws its retarget timer when
    // due (1080) and its look-around timeout (1100); through 1110, before the cow's MOVE of 1118
    let Some(found) = differences(680, 430, 2) else {
        return eprintln!("GENIE_SNAPSHOTS / GENIE_TICK_REC / GENIE_DAT are not set; skipping");
    };
    assert_eq!(found, Vec::<String>::new());
}

#[test]
fn the_cow_s_move_from_a_save_matches_the_game_object_by_object() {
    // updates 682 to 1160: in 797 the tree 9365, let go and flagged to sleep, moves in its own
    // update to Gaia's sleeping list, the last live object (the tree 9357, just woken) taking its
    // place without an update in that tick; in 1118 player 2's MOVE sends the cow 9517: its
    // Artifact action takes the point (slot 8) with a MoveTo sub-action, its unit AI only the
    // point, and the cow walks; in 1164 its step's obstruction query (FUN_14114cc60) finds no
    // hit; through the save after 1164, before its waypoint's look ahead (1165)
    let Some(found) = differences(680, 483, 2) else {
        return eprintln!("GENIE_SNAPSHOTS / GENIE_TICK_REC / GENIE_DAT are not set; skipping");
    };
    assert_eq!(found, Vec::<String>::new());
}

#[test]
fn the_felled_tree_s_wood_from_a_save_matches_the_game_object_by_object() {
    // updates 682 to 1180: 9181's second hit in 1127 fells the tree 9357 (dead in 1128, its
    // falling sprite's facet drawn from the Object stream); in 1168 its Attack ends (code 5) and
    // the Hunt walks to the tree (state 4, a quiet MoveTo to it, in range in 1169), faces it in
    // 1170 (state 6, its moving sprite) and gathers from 1171 on (state 7: the work rate times the
    // task's work value times dt, the wood moving from the tree to its load); in 1165 the cow
    // 9517's step meets Gaia's cow 9519 and the avoidance lets it pass (the overlap does not
    // shrink, the paths have no ignore units, 9519 has not moved since the path was made, and
    // Gaia's units never outrank a player's); every object through the save after 1180
    let Some(found) = differences(680, 500, 2) else {
        return eprintln!("GENIE_SNAPSHOTS / GENIE_TICK_REC / GENIE_DAT are not set; skipping");
    };
    assert_eq!(found, Vec::<String>::new());
}

/// Not pinned: what the slice gets wrong some ticks after a save, for the tick FD-008 works on
/// (`GENIE_SNAPSHOT_START`, default 679; `GENIE_SNAPSHOT_TICKS`, default 1), for player 2 or
/// `GENIE_SNAPSHOT_PLAYER`.
#[test]
fn the_slice_one_tick_after_a_save_report() {
    let start = std::env::var("GENIE_SNAPSHOT_START").map_or(679, |s| s.parse().unwrap());
    let ticks = std::env::var("GENIE_SNAPSHOT_TICKS").map_or(1, |s| s.parse().unwrap());
    let player = std::env::var("GENIE_SNAPSHOT_PLAYER").map_or(2, |s| s.parse().unwrap());
    let Some(found) = differences(start, ticks, player) else {
        return eprintln!("GENIE_SNAPSHOTS / GENIE_TICK_REC / GENIE_DAT are not set; skipping");
    };
    eprintln!(
        "tick {} from the save after {start}: {found:#?}",
        start + ticks
    );
}
