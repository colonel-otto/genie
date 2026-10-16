//! The unit-AI slice against checkpoint word 9. Needs `GENIE_REC_CORPUS` and `GENIE_MASTERS`; skips
//! without them.

mod common;

use common::{corpus, is_scenario, masters, tier1_recordings};
use genie_oracle::{GroupCommand, SAVE_WORDS, compare_first_checkpoint, simulate_word9};

/// MarvinBlue/426048731 player 2 is the corpus's one lane that is idle up to checkpoint 1, and the
/// slice reproduces its word 9 there. It needs the objects that die at the start and player 1's two
/// BUILDs (ticks 135 and 281), whose foundations' unit-AI constructors draw from the shared AI
/// stream; tick 281 is player 2's turn. The BUILDs may create unit groups (`GROUP_COMMANDS`); with
/// villagers as builders they do not.
#[test]
fn marvinblue_player_2_is_reproduced_at_checkpoint_1() {
    let (Some(dir), Some(masters)) = (corpus(), masters()) else {
        return eprintln!("GENIE_REC_CORPUS / GENIE_MASTERS are not set; skipping");
    };
    let data = std::fs::read(dir.join("MarvinBlue/426048731.aoe2record")).unwrap();
    let lanes = simulate_word9(&data, Some(&masters)).unwrap();
    let p2 = lanes.iter().find(|w| w.id == 2).unwrap();
    assert_eq!(p2.unsupported, None);
    assert_eq!(
        p2.group_command,
        Some(GroupCommand {
            tick: 136,
            player: 1,
            command: 0x66
        })
    );
    assert_eq!((p2.simulated, p2.checkpoint), (62248, 62248));
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
/// Every one of them reproduces.
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
    assert_eq!((exact.len(), off.len(), unsupported.len()), (22, 0, 0));
}
