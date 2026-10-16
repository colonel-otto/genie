//! A tick oracle trace (FD-015) against the recording it was played from: at every checkpoint the
//! trace holds the recorded lanes (the state after the tick before) and the recorded world time
//! (that state's time plus the checkpoint tick's dt). Needs `GENIE_TICK_TRACE` (a GRTICK1 file) and
//! `GENIE_TICK_REC` (its recording); skips without them.

mod common;

use genie_oracle::tick_trace::TickTrace;
use genie_rec::de::{self, Op};

#[test]
fn the_trace_holds_every_checkpoint_of_its_recording() {
    let (Ok(trace), Ok(rec)) = (
        std::env::var("GENIE_TICK_TRACE"),
        std::env::var("GENIE_TICK_REC"),
    ) else {
        return eprintln!("GENIE_TICK_TRACE / GENIE_TICK_REC are not set; skipping");
    };
    let trace = TickTrace::decode(&std::fs::read(trace).unwrap()).unwrap();
    let data = std::fs::read(rec).unwrap();
    let chapter = de::chapters(&data).unwrap()[0];
    let (mut tick, mut checked) = (0usize, 0);
    for op in chapter.ops() {
        match op.unwrap() {
            Op::Sync { .. } => tick += 1,
            Op::Checkpoint { dt, checkpoint } => {
                if tick > trace.len() {
                    break;
                }
                let lanes: Vec<_> = checkpoint
                    .lanes
                    .iter()
                    .filter(|l| l.iter().any(|&w| w != 0))
                    .collect();
                let traced = trace.lanes(tick - 1);
                assert_eq!(traced.len(), lanes.len(), "lanes at tick {tick}");
                for (t, r) in traced.iter().zip(&lanes) {
                    assert_eq!(t, *r, "tick {tick}");
                }
                assert_eq!(
                    trace.world_time(tick - 1) + dt,
                    checkpoint.world_time,
                    "world time at tick {tick}"
                );
                checked += 1;
                tick += 1;
            }
            _ => {}
        }
    }
    eprintln!("{checked} checkpoints in a trace of {} ticks", trace.len());
    assert!(checked > 0);
}

/// The slice against the tick oracle, tick by tick over the whole trace: MarvinBlue/426048731 player
/// 2 (lane 1, idle up to checkpoint 1) and the first tick where the slice's block leaves the game's,
/// with the words that differ there. Needs the trace of that recording in `GENIE_TICK_TRACE` and `GENIE_MASTERS`.
#[test]
fn the_slice_follows_the_game_up_to_a_named_tick() {
    let (Ok(trace), Ok(rec), Some(masters)) = (
        std::env::var("GENIE_TICK_TRACE"),
        std::env::var("GENIE_TICK_REC"),
        common::masters(),
    ) else {
        return eprintln!(
            "GENIE_TICK_TRACE / GENIE_TICK_REC / GENIE_MASTERS are not set; skipping"
        );
    };
    if !rec.ends_with("426048731.aoe2record") {
        return eprintln!("the slice's lane is MarvinBlue/426048731's player 2; skipping");
    }
    let trace = TickTrace::decode(&std::fs::read(trace).unwrap()).unwrap();
    let data = std::fs::read(rec).unwrap();
    let slice = genie_oracle::blocks_per_tick(&data, &[2], trace.len(), Some(&masters)).unwrap();
    let differing = |t: usize| -> Vec<usize> {
        (0..11)
            .filter(|&w| slice[t][0][w] != trace.lanes(t)[1][w])
            .collect()
    };
    let first = (0..slice.len()).find(|&t| !differing(t).is_empty());
    let words = first.map(differing).unwrap_or_default();
    eprintln!(
        "first differing tick: {first:?}, words {words:?} (slice {:?}, game {:?})",
        first.map(|t| words.iter().map(|&w| slice[t][0][w]).collect::<Vec<_>>()),
        first.map(|t| words
            .iter()
            .map(|&w| trace.lanes(t)[1][w])
            .collect::<Vec<_>>())
    );
    // the slice holds the whole block, through player 1's BUILDs at ticks 135 and 281 (their
    // foundations' unit-AI constructors draw three AI values each), until player 2's own first
    // command: a villager QUEUE (0x81) at its town centre in tick 553
    assert_eq!((first, words), (Some(553), vec![1, 9]));
}

/// The slice's word-9 terms per object against the game's after a tick (MarvinBlue/426048731
/// player 2): `GENIE_WORD9_TERMS` names the oracle's dump (id, top action type, state, retarget per
/// line, e.g. `aoe2de-exe-re/artifacts/playback/426048731_p2_word9_tick0.txt`) and
/// `GENIE_WORD9_TICK` the 0-based tick it was taken after (0 by default). Reports the objects that
/// differ.
#[test]
fn the_slices_word9_terms_against_the_games_after_a_tick() {
    let (Ok(terms), Ok(rec), Some(masters)) = (
        std::env::var("GENIE_WORD9_TERMS"),
        std::env::var("GENIE_TICK_REC"),
        common::masters(),
    ) else {
        return eprintln!(
            "GENIE_WORD9_TERMS / GENIE_TICK_REC / GENIE_MASTERS are not set; skipping"
        );
    };
    let game: Vec<genie_oracle::Word9Terms> = std::fs::read_to_string(terms)
        .unwrap()
        .lines()
        .filter(|l| !l.starts_with('#'))
        .map(|l| {
            let v: Vec<i64> = l.split_whitespace().map(|x| x.parse().unwrap()).collect();
            genie_oracle::Word9Terms {
                id: v[0] as u32,
                action: v[1] as i32,
                state: v[2] as u8,
                retarget: v[3] as i32,
            }
        })
        .collect();
    let tick: usize = std::env::var("GENIE_WORD9_TICK").map_or(0, |t| t.parse().unwrap());
    let data = std::fs::read(rec).unwrap();
    let slice = genie_oracle::word9_terms(&data, 2, tick + 1, Some(&masters)).unwrap();
    let by_id = |v: &[genie_oracle::Word9Terms]| {
        v.iter()
            .map(|t| (t.id, *t))
            .collect::<std::collections::BTreeMap<_, _>>()
    };
    let (s, g) = (by_id(&slice), by_id(&game));
    let only_game: Vec<_> = g.keys().filter(|k| !s.contains_key(k)).collect();
    let only_slice: Vec<_> = s.keys().filter(|k| !g.contains_key(k)).collect();
    let differing: Vec<_> = s
        .iter()
        .filter(|(k, t)| g.get(k).is_some_and(|x| x != *t))
        .collect();
    eprintln!("objects only the game has: {only_game:?}; only the slice: {only_slice:?}");
    for (k, t) in &differing {
        eprintln!("object {k}: slice {t:?}, game {:?}", g[k]);
    }
    eprintln!("{} common objects differ", differing.len());
    let order: Vec<u32> = slice.iter().map(|t| t.id).collect();
    let game_order: Vec<u32> = game.iter().map(|t| t.id).collect();
    eprintln!("same list order: {}", order == game_order);
}
