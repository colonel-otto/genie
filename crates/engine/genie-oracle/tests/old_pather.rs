//! The old short pather (genie-core `hull_path`) against the game's own calls of slot 0x3e8 in
//! MarvinBlue/426048731 (`load_oracle/probes/_obspath.py`): the drop-site searches of six
//! villagers to the town centre 9149 in update 3679 (probe tick 2999) and 4074 (3394), each in
//! the world of the save before its update with the villager at its place then. Needs
//! `GENIE_SNAPSHOTS` (the saves and dumps), `GENIE_TICK_REC` and `GENIE_DAT`.

mod common;

use std::path::PathBuf;

/// (villager, its place, the game's path) of one update's calls; all within 0.1 of the TC 9149.
type Call = (i32, [f32; 2], &'static [[f32; 2]]);

const UPDATE_3679: &[Call] = &[
    (
        10036,
        [108.085_14, 44.347_122],
        &[
            [108.085_14, 44.347_122],
            [107.285_15, 44.291_824],
            [106.2, 44.8],
        ],
    ),
    (
        9179,
        [108.612_91, 45.521_812],
        &[
            [108.612_91, 45.521_812],
            [108.613, 45.908_203],
            [106.2, 47.2],
        ],
    ),
    (
        9173,
        [107.685_14, 44.691_925],
        &[[107.685_14, 44.691_925], [106.2, 44.8]],
    ),
    (
        10033,
        [108.212_906, 45.5082],
        &[
            [108.212_906, 45.5082],
            [108.212_906, 45.921_913],
            [106.2, 47.2],
        ],
    ),
    (
        10029,
        [107.698_75, 45.091_927],
        &[
            [107.698_75, 45.091_927],
            [107.300_82, 45.245],
            [106.2, 47.2],
        ],
    ),
    (
        9171,
        [107.767_56, 45.491_924],
        &[
            [107.767_56, 45.491_924],
            [105.630_646, 47.575_527],
            [103.907_16, 47.037_163],
        ],
    ),
];

const UPDATE_4074: &[Call] = &[
    (
        10029,
        [108.105_45, 44.371_895],
        &[
            [108.105_45, 44.371_895],
            [108.105_446, 44.205_498],
            [107.305_46, 44.2054],
            [106.2, 44.8],
        ],
    ),
    (
        10036,
        [107.705_45, 44.6055],
        &[[107.705_45, 44.6055], [106.2, 44.8]],
    ),
    (
        10033,
        [108.495_24, 45.513_596],
        &[
            [108.495_24, 45.513_596],
            [108.495_24, 45.928_82],
            [106.2, 47.2],
        ],
    ),
    (
        9173,
        [107.694_25, 45.0055],
        &[
            [107.694_25, 45.0055],
            [107.295_78, 45.005_478],
            [106.2, 47.2],
        ],
    ),
    (
        9179,
        [108.095_24, 45.529_587],
        &[
            [108.095_24, 45.529_587],
            [107.812_39, 45.812_43],
            [106.2, 47.2],
        ],
    ),
    (
        9171,
        [107.695_77, 45.405_58],
        &[
            [107.695_77, 45.405_58],
            [105.672_25, 47.558_23],
            [103.908_18, 47.039_61],
        ],
    ),
];

fn check(save_tick: usize, calls: &[Call]) -> Option<Vec<String>> {
    let dir = PathBuf::from(std::env::var_os("GENIE_SNAPSHOTS")?);
    let rec = std::fs::read(std::env::var_os("GENIE_TICK_REC")?).ok()?;
    let masters = common::masters()?;
    let save = std::fs::read(dir.join(format!("{save_tick}.aoe2record"))).ok()?;
    let read = |ext: &str| std::fs::read_to_string(dir.join(format!("{save_tick}.{ext}"))).ok();
    let mut mesh = read("mesh");
    if mesh.is_none() {
        let raw = std::fs::read(dir.join(format!("{save_tick}.mesh.gz"))).ok()?;
        let mut text = String::new();
        let _ =
            std::io::Read::read_to_string(&mut flate2::read::GzDecoder::new(&raw[..]), &mut text);
        mesh = Some(text);
    }
    let (registry, tiles, pools) = (read("registry"), read("tiles"), read("pools"));
    let dumps = genie_oracle::Dumps {
        mesh: mesh.as_deref(),
        registry: registry.as_deref(),
        tiles: tiles.as_deref(),
        pools: pools.as_deref(),
    };
    let queries: Vec<_> = calls
        .iter()
        .map(|&(id, from, _)| (id, from, 9149, 0.1))
        .collect();
    let got =
        genie_oracle::old_pather_paths(&save, &rec, save_tick, Some(&masters), dumps, &queries)
            .unwrap();
    let mut wrong = Vec::new();
    for (&(id, _, want), got) in calls.iter().zip(got) {
        match got {
            Ok((1, path)) if path == want => {}
            other => wrong.push(format!("{id}: the game 1 {want:?}, the slice {other:?}")),
        }
    }
    Some(wrong)
}

#[test]
fn the_drop_site_searches_of_update_3679_find_the_game_s_paths() {
    let Some(wrong) = check(3677, UPDATE_3679) else {
        return eprintln!("GENIE_SNAPSHOTS / GENIE_TICK_REC / GENIE_DAT are not set; skipping");
    };
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn the_drop_site_searches_of_update_4074_find_the_game_s_paths() {
    let Some(wrong) = check(4072, UPDATE_4074) else {
        return eprintln!("GENIE_SNAPSHOTS / GENIE_TICK_REC / GENIE_DAT are not set; skipping");
    };
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}
