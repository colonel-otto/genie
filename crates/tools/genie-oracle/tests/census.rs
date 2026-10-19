//! The census run (FD-025): a recording from its header, nothing forced, each player's first
//! unmodelled stop and first recorded checkpoint that differs. Skips without `GENIE_DAT` and the
//! recordings' directory `GENIE_RECORDINGS` (`unobfuscated/recorded-games`).

mod common;

use std::path::PathBuf;

use genie_oracle::census::{CensusEnd, census_run};

fn recording(name: &str) -> Option<Vec<u8>> {
    let dir = PathBuf::from(std::env::var_os("GENIE_RECORDINGS")?);
    std::fs::read(dir.join(name)).ok()
}

/// MarvinBlue/426048731 replays fully: every checkpoint in its first 2000 updates holds for every
/// player, and no player stops.
#[test]
fn a_recording_the_slice_replays_holds_its_checkpoints() {
    let (Some(rec), Some(masters)) = (
        recording("MarvinBlue/426048731.aoe2record"),
        common::masters(),
    ) else {
        return eprintln!("GENIE_RECORDINGS / GENIE_DAT are not set; skipping");
    };
    let run = census_run(&rec, Some(&masters), Some(2000)).unwrap();
    assert_eq!((run.updates, run.end), (2000, CensusEnd::Limit));
    assert_eq!(
        run.checkpoints, 4,
        "the checkpoints after 499, 999, 1499 and 1999 syncs"
    );
    for (p, player) in run.players.iter().enumerate() {
        assert_eq!(player.stop, None, "player {p}");
        assert_eq!(player.mismatch, None, "player {p}");
    }
    // Gaia has no lane in the checkpoints
    assert_eq!(run.players[0].checkpoints_held, 0);
    assert!(run.players[1..].iter().all(|p| p.checkpoints_held == 4));
}

/// records/425906646 to update 4200: player 6 stops at update 4153 on the `SeekShelter` command
/// (the free run's stop), the others run on; the checkpoints to 2000 syncs hold (FD-008).
#[test]
fn a_recording_s_census_names_each_player_s_first_stop() {
    let (Some(rec), Some(masters)) = (recording("records/425906646.aoe2record"), common::masters())
    else {
        return eprintln!("GENIE_RECORDINGS / GENIE_DAT are not set; skipping");
    };
    let run = census_run(&rec, Some(&masters), Some(4200)).unwrap();
    let stop = run.players[6].stop.as_ref().expect("player 6 stops");
    assert_eq!(stop.update, 4153);
    assert_eq!(
        stop.reason,
        "a command the slice does not model: SeekShelter"
    );
    for player in &run.players[1..] {
        assert!(player.mismatch.as_ref().is_none_or(|m| 2000 < m.syncs));
        assert!(4 <= player.checkpoints_held);
    }
}
