//! The obstruction records the slice builds from a save, against the game's own obstruction
//! manager at the same tick: `GENIE_OBSTRUCTIONS` names the tick oracle's dump
//! (`load_oracle/probes/obstructions.py` after 680 updates of MarvinBlue/426048731, in
//! `aoe2de-exe-re/artifacts/playback/obstructions_426048731_679.txt`), `GENIE_SNAPSHOTS` the
//! directory with the save of that tick (`679.aoe2record`). Skips without them.

use std::collections::BTreeMap;

type Records = BTreeMap<i32, (u8, [f32; 4])>;

/// The dump's records: `ID type KIND flags .. box (X0, Y0, X1, Y1)` per line.
fn game_records(text: &str) -> Records {
    text.lines()
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            let id: i32 = words.next()?.parse().ok()?;
            (words.next()? == "type").then_some(())?;
            let kind: u8 = words.next()?.parse().ok()?;
            let inside = line.split("box (").nth(1)?.trim_end_matches(')');
            let v: Vec<f32> = inside.split(", ").map(|f| f.parse().unwrap()).collect();
            Some((id, (kind, [v[0], v[1], v[2], v[3]])))
        })
        .collect()
}

#[test]
fn the_slice_s_obstruction_records_are_the_game_s() {
    let (Some(dump), Some(dir)) = (
        std::env::var_os("GENIE_OBSTRUCTIONS"),
        std::env::var_os("GENIE_SNAPSHOTS"),
    ) else {
        return eprintln!("GENIE_OBSTRUCTIONS / GENIE_SNAPSHOTS are not set; skipping");
    };
    let game = game_records(&std::fs::read_to_string(dump).unwrap());
    let save = std::fs::read(std::path::Path::new(&dir).join("679.aoe2record")).unwrap();
    let spatial = genie_oracle::spatial_from_save(&save).unwrap();
    assert_eq!(spatial.map, [120, 120]);
    let ours: Records = spatial
        .records()
        .map(|(id, kind, bounds)| (id, (kind, bounds)))
        .collect();
    let mut differences: Vec<String> = Vec::new();
    for (id, g) in &game {
        match ours.get(id) {
            None => differences.push(format!("{id}: only in the game {g:?}")),
            Some(o) if o != g => differences.push(format!("{id}: slice {o:?} game {g:?}")),
            Some(_) => {}
        }
    }
    for (id, o) in ours.iter().filter(|(id, _)| !game.contains_key(id)) {
        differences.push(format!("{id}: only in the slice {o:?}"));
    }
    assert_eq!(game.len(), 7488);
    assert!(
        differences.is_empty(),
        "{} differences, the first: {:#?}",
        differences.len(),
        &differences[..differences.len().min(20)]
    );
}
