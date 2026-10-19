//! The obstruction records the slice builds from a save, against the game's own obstruction
//! manager at the same tick: `GENIE_OBSTRUCTIONS` names the tick oracle's dump
//! (`load_oracle/probes/obstructions.py` after 680 updates of MarvinBlue/426048731, in
//! `aoe2de-exe-re/artifacts/playback/obstructions_426048731_679.txt`), `GENIE_SNAPSHOTS` the
//! directory with the save of that tick (`679.aoe2record`). The records' grid cells are checked
//! too. Skips without them.

// the dumps' numbers fit the game's widths; a test runs the whole comparison
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::too_many_lines
)]

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

/// The dump's grid cells: `ID .. level LEVEL cell (X,Y) ..` per line.
fn game_cells(text: &str) -> BTreeMap<i32, (i32, i32, i32)> {
    text.lines()
        .filter_map(|line| {
            let id: i32 = line.split_whitespace().next()?.parse().ok()?;
            let level: i32 = line
                .split(" level ")
                .nth(1)?
                .split(' ')
                .next()?
                .parse()
                .ok()?;
            let cell = line.split(" cell (").nth(1)?.split(')').next()?;
            let (x, y) = cell.split_once(',')?;
            Some((id, (level, x.parse().ok()?, y.parse().ok()?)))
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
    // the grid cell each record is filed in
    let cells = game_cells(
        &std::fs::read_to_string(std::env::var_os("GENIE_OBSTRUCTIONS").unwrap()).unwrap(),
    );
    for (id, cell) in spatial.record_cells() {
        if cells.get(&id).is_some_and(|g| *g != cell) {
            differences.push(format!("{id}: cell slice {cell:?} game {:?}", cells[&id]));
        }
    }
    assert_eq!(game.len(), 7488);
    assert!(
        differences.is_empty(),
        "{} differences, the first: {:#?}",
        differences.len(),
        &differences[..differences.len().min(20)]
    );
}

/// The dump of the obstruction manager's tile grid and the map's per-tile counts
/// (`load_oracle/probes/tilegrid.py` after 680 updates): width and height, the grid words row by
/// row, the terrain restrictions' masks, then per tile `tall:objects`.
struct GameTiles {
    width: usize,
    words: Vec<u64>,
    masks: Vec<u8>,
    counts: Vec<(u8, u8)>,
}

fn game_tiles(text: &str) -> GameTiles {
    let mut lines = text.lines();
    let size: Vec<usize> = lines
        .next()
        .unwrap()
        .split(' ')
        .map(|v| v.parse().unwrap())
        .collect();
    let (width, height) = (size[0], size[1]);
    let words = lines
        .by_ref()
        .take(height)
        .flat_map(|l| l.split(' ').map(|v| u64::from_str_radix(v, 16).unwrap()))
        .collect();
    let masks = lines
        .next()
        .unwrap()
        .split(' ')
        .skip(1)
        .map(|v| u8::from_str_radix(v, 16).unwrap())
        .collect();
    let counts = lines
        .take(height)
        .flat_map(|l| {
            l.split(' ').map(|c| {
                let (a, b) = c.split_once(':').unwrap();
                (a.parse().unwrap(), b.parse().unwrap())
            })
        })
        .collect();
    GameTiles {
        width,
        words,
        masks,
        counts,
    }
}

#[test]
fn the_slice_s_tile_grid_and_counts_are_the_game_s() {
    let (Some(dump), Some(dir), Some(dat)) = (
        std::env::var_os("GENIE_TILEGRID"),
        std::env::var_os("GENIE_SNAPSHOTS"),
        std::env::var_os("GENIE_DAT"),
    ) else {
        return eprintln!("GENIE_TILEGRID / GENIE_SNAPSHOTS / GENIE_DAT are not set; skipping");
    };
    let game = game_tiles(&std::fs::read_to_string(dump).unwrap());
    let save = std::fs::read(std::path::Path::new(&dir).join("679.aoe2record")).unwrap();
    let dat = genie_dat::de::DatFile::read_from(std::fs::File::open(dat).unwrap()).unwrap();
    let terrain = genie_oracle::TerrainTable::from_dat(&dat);
    assert_eq!(terrain.masks(), &game.masks[..terrain.masks().len()]);
    let spatial = genie_oracle::spatial_with_tiles(&save, &terrain).unwrap();
    let tiles = spatial.tiles();
    let mut differences = Vec::new();
    for (i, &word) in game.words.iter().enumerate() {
        let (x, y) = ((i % game.width) as i32, (i / game.width) as i32);
        let ours = tiles.word(x, y);
        if ours != Some(word) {
            differences.push(format!("({x},{y}) word slice {ours:x?} game {word:x}"));
        }
        let counts = (tiles.tall(x, y), tiles.objects(x, y));
        let (o, n) = game.counts[i];
        if counts != (Some(o), Some(n)) {
            differences.push(format!("({x},{y}) counts slice {counts:?} game ({o}, {n})"));
        }
    }
    assert!(
        differences.is_empty(),
        "{} differences, the first: {:#?}",
        differences.len(),
        &differences[..differences.len().min(20)]
    );
}
