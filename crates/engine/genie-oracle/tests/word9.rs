//! The unit-AI slice against checkpoint word 9. Needs `GENIE_REC_CORPUS`; skips without it.

mod common;

use common::{corpus, is_scenario, tier1_recordings};
use genie_oracle::{GroupCommand, SAVE_WORDS, compare_first_checkpoint, simulate_word9};

/// MarvinBlue/426048731 player 2 is the corpus's one lane that is idle up to checkpoint 1. The
/// slice models all its units (no `unsupported`), but word 9 is not reproduced yet. Player 1's
/// commands before the checkpoint (a villager BUILD at tick 137, single-scout MOVEs from tick 421)
/// may or may not create unit groups; villagers and single units likely do not.
#[test]
fn marvinblue_player_2_is_modelled_but_not_reproduced_yet() {
    let Some(dir) = corpus() else {
        return eprintln!("GENIE_REC_CORPUS is not set; skipping");
    };
    let data = std::fs::read(dir.join("MarvinBlue/426048731.aoe2record")).unwrap();
    let lanes = simulate_word9(&data).unwrap();
    let p2 = lanes.iter().find(|w| w.id == 2).unwrap();
    assert_eq!(p2.unsupported, None);
    assert_eq!(p2.checkpoint, 62248);
    assert_eq!(
        p2.group_command,
        Some(GroupCommand {
            tick: 137,
            player: 1,
            command: 0x66
        })
    );
    // the current model's value, pinned so that a model change is noticed; the cow's Artifact
    // action adds 118, the remaining 881 too many are open
    assert_eq!(p2.simulated, 63129);
}

/// MarvinBlue/426048869 player 6 (three villagers and a goat, idle up to the checkpoint): the goat's
/// default Artifact action, type 107 in state 10, is the top action word 9 counts.
#[test]
fn an_idle_player_with_livestock_is_reproduced() {
    let Some(dir) = corpus() else {
        return eprintln!("GENIE_REC_CORPUS is not set; skipping");
    };
    let data = std::fs::read(dir.join("MarvinBlue/426048869.aoe2record")).unwrap();
    let lanes = simulate_word9(&data).unwrap();
    let p6 = lanes.iter().find(|w| w.id == 6).unwrap();
    assert_eq!((p6.simulated, p6.checkpoint), (3938, 3938));
}

/// Word 9 on every tier-1 lane whose ten save words already match the first checkpoint, the lanes
/// whose players changed nothing the save words see. Pinned, so that a model change is noticed.
/// Open: the five lanes with a Scout Cavalry or one of them off by -215 (a single AI draw), and one
/// transport-ship unit AI (type 11) the slice does not model.
#[test]
fn tier1_lanes_with_matching_save_words() {
    let Some(dir) = corpus() else {
        return eprintln!("GENIE_REC_CORPUS is not set; skipping");
    };
    let (mut exact, mut off, mut unsupported) = (Vec::new(), Vec::new(), 0);
    for (path, data) in tier1_recordings(&dir) {
        if is_scenario(&data) {
            continue;
        }
        let (Ok(compared), Ok(word9)) = (compare_first_checkpoint(&data), simulate_word9(&data))
        else {
            continue;
        };
        let name = path.strip_prefix(&dir).unwrap().display().to_string();
        for (c, w) in compared.iter().zip(&word9) {
            if SAVE_WORDS
                .iter()
                .any(|&k| c.from_save[k] != c.checkpoint[k])
            {
                continue;
            }
            if w.unsupported.is_some() {
                unsupported += 1;
            } else if w.simulated == w.checkpoint {
                exact.push(format!("{name} p{}", w.id));
            } else {
                off.push(format!(
                    "{name} p{}: {}",
                    w.id,
                    w.checkpoint as i64 - w.simulated as i64
                ));
            }
        }
    }
    eprintln!("exact {exact:#?}\noff {off:#?}\nunsupported {unsupported}");
    assert_eq!((exact.len(), off.len(), unsupported), (14, 5, 1));
}
