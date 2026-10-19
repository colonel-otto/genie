//! Recordings' header saves against the checkpoints in their own bodies. Needs
//! `GENIE_REC_CORPUS`; skips without it.

mod common;

use common::{corpus, is_scenario, tier1_recordings};
use genie_oracle::{Error, SAVE_WORDS, compare_first_checkpoint};

fn differing(from_save: &[u32; 11], checkpoint: &[u32; 11]) -> Vec<usize> {
    (0..11).filter(|&k| from_save[k] != checkpoint[k]).collect()
}

#[test]
fn marvinblue_header_save_reproduces_the_first_checkpoint_of_player_2() {
    let Some(dir) = corpus() else {
        return eprintln!("GENIE_REC_CORPUS is not set; skipping");
    };
    let data = std::fs::read(dir.join("MarvinBlue/426048731.aoe2record")).unwrap();
    let players = compare_first_checkpoint(&data).unwrap();
    assert_eq!(players.iter().map(|p| p.id).collect::<Vec<_>>(), [1, 2]);
    let [p1, p2] = &players[..] else {
        unreachable!()
    };
    // Player 2's state is unchanged at the first checkpoint, apart from word 9.
    assert_eq!(differing(&p2.from_save, &p2.checkpoint), [9]);
    assert_eq!(
        p2.from_save,
        [
            0,
            700,
            11230,
            126,
            150,
            0,
            63,
            771_100,
            2,
            p2.from_save[9],
            586_462
        ]
    );
    // Player 1 has spent 250 resources and has 4 more objects by then: that needs the simulation.
    assert_eq!(
        differing(&p1.from_save, &p1.checkpoint),
        [1, 2, 3, 4, 6, 7, 9, 10]
    );
    assert_eq!((p1.from_save[1], p1.checkpoint[1]), (700, 450));
    assert_eq!((p1.from_save[6], p1.checkpoint[6]), (66, 70));
}

/// How many players of the tier-1 corpus the save alone reproduces. Only reported: a player who
/// acts in the first 500 ticks needs the simulation. Words 0, 5 and 8 (sleeping count, waypoints,
/// player id) must match for every player, whatever they did, except in scenario games: there the
/// header holds the scenario's placed objects, and the scenario start replaces them before the first
/// checkpoint (`MarvinBlue/426048681`: player 1 has 193 live and 2 sleeping objects in the header,
/// 40 and 0 at checkpoint 1). Scenarios are only counted.
#[test]
fn tier1_corpus_report() {
    let Some(dir) = corpus() else {
        return eprintln!("GENIE_REC_CORPUS is not set; skipping");
    };
    let mut outcomes = std::collections::BTreeMap::<String, usize>::new();
    let mut lanes = std::collections::BTreeMap::<String, usize>::new();
    for (path, data) in tier1_recordings(&dir) {
        if is_scenario(&data) {
            *outcomes
                .entry("scenario, not compared".to_string())
                .or_default() += 1;
            continue;
        }
        let outcome = match compare_first_checkpoint(&data) {
            Ok(players) => {
                for p in players {
                    for k in [0, 5, 8] {
                        assert_eq!(
                            p.from_save[k],
                            p.checkpoint[k],
                            "{} player {} word {k}",
                            path.display(),
                            p.id
                        );
                    }
                    let off: Vec<usize> = differing(&p.from_save, &p.checkpoint)
                        .into_iter()
                        .filter(|k| SAVE_WORDS.contains(k))
                        .collect();
                    *lanes.entry(format!("{off:?}")).or_default() += 1;
                }
                "compared".to_string()
            }
            Err(Error::Rec(genie_rec::de::Error::Header(genie_save::Error::Unsupported {
                what,
                ..
            }))) => {
                format!("unsupported: {what}")
            }
            Err(Error::NoCheckpoint) => "no checkpoint".to_string(),
            Err(e) => panic!("{}: {e}", path.display()),
        };
        *outcomes.entry(outcome).or_default() += 1;
    }
    eprintln!("recordings: {outcomes:?}");
    eprintln!("players by differing save words: {lanes:?}");
}
