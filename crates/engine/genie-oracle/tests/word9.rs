//! The unit-AI slice against checkpoint word 9. Needs `GENIE_REC_CORPUS` and `GENIE_MASTERS`; skips
//! without them.

mod common;

use common::{corpus, is_scenario, masters, tier1_recordings};
use genie_oracle::{GroupCommand, SAVE_WORDS, compare_first_checkpoint, simulate_word9};

/// MarvinBlue/426048731 player 2 is the corpus's one lane that is idle up to checkpoint 1. The
/// slice models all its units (no `unsupported`), but word 9 is not reproduced yet: against the tick
/// oracle the slice holds the whole block up to tick 280; in tick 281 player 1's second BUILD places
/// a house foundation whose unit-AI constructor draws three AI values before player 2's units update
/// (`tests/tick_trace_corpus.rs`).
#[test]
fn marvinblue_player_2_is_modelled_but_not_reproduced_yet() {
    let (Some(dir), Some(masters)) = (corpus(), masters()) else {
        return eprintln!("GENIE_REC_CORPUS / GENIE_MASTERS are not set; skipping");
    };
    let data = std::fs::read(dir.join("MarvinBlue/426048731.aoe2record")).unwrap();
    let lanes = simulate_word9(&data, Some(&masters)).unwrap();
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
    // action and the objects that die at the start are in, the remaining 188 too many are open
    assert_eq!(p2.simulated, 62436);
}

/// MarvinBlue/426048869 player 6 (three villagers and a goat, idle up to the checkpoint): the goat's
/// default Artifact action, type 107 in state 10, is the top action word 9 counts.
#[test]
fn an_idle_player_with_livestock_is_reproduced() {
    let (Some(dir), Some(masters)) = (corpus(), masters()) else {
        return eprintln!("GENIE_REC_CORPUS / GENIE_MASTERS are not set; skipping");
    };
    let data = std::fs::read(dir.join("MarvinBlue/426048869.aoe2record")).unwrap();
    let lanes = simulate_word9(&data, Some(&masters)).unwrap();
    let p6 = lanes.iter().find(|w| w.id == 6).unwrap();
    assert_eq!((p6.simulated, p6.checkpoint), (3938, 3938));
}

/// Word 9 on every tier-1 lane whose ten save words already match the first checkpoint, the lanes
/// whose players changed nothing the save words see. Pinned, so that a model change is noticed.
/// Open: the five lanes off (MarvinBlue/426048731 player 2's is another player's BUILD; the others
/// have a Scout Cavalry or are off by a single AI draw), and the two the slice does not model.
#[test]
fn tier1_lanes_with_matching_save_words() {
    let (Some(dir), Some(masters)) = (corpus(), masters()) else {
        return eprintln!("GENIE_REC_CORPUS / GENIE_MASTERS are not set; skipping");
    };
    let (mut exact, mut off, mut unsupported) = (Vec::new(), Vec::new(), Vec::new());
    for (path, data) in tier1_recordings(&dir) {
        if is_scenario(&data) {
            continue;
        }
        let (Ok(compared), Ok(word9)) = (
            compare_first_checkpoint(&data),
            simulate_word9(&data, Some(&masters)),
        ) else {
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
            if let Some((tick, object, why)) = w.unsupported {
                unsupported.push(format!(
                    "{name} p{}: tick {tick}, object {object}: {}",
                    w.id, why.0
                ));
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
    eprintln!("exact {exact:#?}\noff {off:#?}\nunsupported {unsupported:#?}");
    assert_eq!((exact.len(), off.len(), unsupported.len()), (15, 5, 2));
}
