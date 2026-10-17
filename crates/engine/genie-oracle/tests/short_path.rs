//! The short pather, against the game's own: `GENIE_SHORTQ` names a run of the game's
//! `FUN_140f9e540` on pseudo-random queries after 680 updates of MarvinBlue/426048731
//! (`load_oracle/probes/shortpath_queries.py`, one sequence: each search changes the mesh's free
//! list), `GENIE_MESH` the game's mesh before them (`load_oracle/probes/mesh.py`), `GENIE_SNAPSHOTS`
//! the save of that tick (the tile words) and `GENIE_DAT` the data file (restriction 7's mask).
//! Skips without them.

use genie_core::short_path::{Call, Entry, Listed, Pather, ShortPath, Target, to_path};

fn f32_of(hex: &str) -> f32 {
    f32::from_bits(u32::from_str_radix(hex, 16).unwrap())
}

#[test]
fn the_slice_s_short_paths_are_the_game_s() {
    let (Some(queries), Some(mesh), Some(dir), Some(dat)) = (
        std::env::var_os("GENIE_SHORTQ"),
        std::env::var_os("GENIE_MESH"),
        std::env::var_os("GENIE_SNAPSHOTS"),
        std::env::var_os("GENIE_DAT"),
    ) else {
        return eprintln!(
            "GENIE_SHORTQ / GENIE_MESH / GENIE_SNAPSHOTS / GENIE_DAT are not set; skipping"
        );
    };
    let mut mesh = genie_oracle::mesh_from_dump(&std::fs::read_to_string(mesh).unwrap()).unwrap();
    let save = std::fs::read(std::path::Path::new(&dir).join("679.aoe2record")).unwrap();
    let dat = genie_dat::de::DatFile::read_from(std::fs::File::open(dat).unwrap()).unwrap();
    let terrain = genie_oracle::TerrainTable::from_dat(&dat);
    let spatial = genie_oracle::spatial_with_tiles(&save, &terrain).unwrap();
    let tiles = spatial.tiles();
    let words = |x: u32, y: u32| tiles.word(x as i32, y as i32).unwrap_or(0);
    let text = std::fs::read_to_string(queries).unwrap();
    let lines: Vec<&str> = text
        .lines()
        .take_while(|l| !l.starts_with("mesh "))
        .collect();
    let mut pather = Pather::default();
    let (mut checked, mut wrong) = (0, Vec::new());
    for pair in lines.chunks(2) {
        let q: Vec<&str> = pair[0].split_whitespace().collect();
        let restriction: usize = q[7].parse().unwrap();
        let mask = terrain.masks()[restriction];
        let (sx, sy, gx, gy) = (f32_of(q[1]), f32_of(q[2]), f32_of(q[3]), f32_of(q[4]));
        let (radius, range) = (f32_of(q[5]), f32_of(q[6]));
        let target = q.iter().position(|&w| w == "target").map(|i| Target {
            id: q[i + 1].parse().unwrap(),
            position: [f32_of(q[i + 2]), f32_of(q[i + 3])],
            half_size: [f32_of(q[i + 4]), f32_of(q[i + 5])],
        });
        let listed = q
            .iter()
            .position(|&w| w == "ignore")
            .map_or_else(Vec::new, |i| {
                q[i + 1..]
                    .iter()
                    .map(|w| {
                        let (id, shape) = w.split_once(':').unwrap();
                        let shape: u32 = shape.parse().unwrap();
                        Listed {
                            id: id.parse().unwrap(),
                            shape: (shape != u32::MAX).then_some(shape),
                        }
                    })
                    .collect()
            });
        let call = Call {
            from: [sx, sy],
            to: [gx, gy],
            radius,
            range,
            target,
            listed,
            map: spatial.map,
            mask,
        };
        let empty = |result| {
            Ok(ShortPath {
                result,
                points: Vec::new(),
            })
        };
        let got = match call.entry() {
            Entry::Search { query, ignore } => pather.search(&mut mesh, &words, &query, &ignore),
            Entry::InRange => empty(0),
            Entry::Invalid => empty(4),
        };
        let r: Vec<&str> = pair[1].split_whitespace().collect();
        let result: u8 = r[1].parse().unwrap();
        let game: Vec<(u32, u32)> = r[2..]
            .iter()
            .map(|w| {
                let (x, y) = w.split_once(',').unwrap();
                (
                    u32::from_str_radix(x, 16).unwrap(),
                    u32::from_str_radix(y, 16).unwrap(),
                )
            })
            .collect();
        let ours = got.map(|sp| {
            let path = to_path(&sp.points);
            let pts: Vec<(u32, u32)> = path
                .points
                .iter()
                .map(|p| (p[0].to_bits(), p[2].to_bits()))
                .collect();
            (sp.result, pts)
        });
        match ours {
            Ok(o) if o == (result, game.clone()) => {}
            other => wrong.push(format!(
                "{}: game {result} {game:x?}, slice {other:x?}",
                pair[0]
            )),
        }
        checked += 1;
    }
    assert!(
        wrong.is_empty(),
        "{} of {checked} queries differ, the first: {:#?}",
        wrong.len(),
        &wrong[..wrong.len().min(8)]
    );
    eprintln!("{checked} queries reproduce");
    // the mesh after the sequence: each search's temporary shapes took and gave back free slots
    let after: String = text
        .lines()
        .skip_while(|l| !l.starts_with("mesh "))
        .map(|l| format!("{l}\n"))
        .collect();
    let game = genie_oracle::mesh_from_dump(&after).unwrap();
    assert_eq!(mesh.free, game.free, "the free list's head");
    assert!(mesh.shapes == game.shapes, "the shape slots");
    assert!(mesh.tiles == game.tiles, "the tile lists");
}
