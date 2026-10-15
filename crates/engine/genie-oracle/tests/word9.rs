//! The unit-AI slice against checkpoint word 9. Needs `GENIE_REC_CORPUS`; skips without it.

use genie_oracle::{GroupCommand, simulate_word9};
use std::path::PathBuf;

fn corpus() -> Option<PathBuf> {
    std::env::var_os("GENIE_REC_CORPUS").map(PathBuf::from)
}

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
    // the current model's value, pinned so that a model change is noticed
    assert_eq!(p2.simulated, 63011);
}
