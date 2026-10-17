//! The tile pathfinder's grid the slice builds from a save, against the game's own planner at the
//! same tick: `GENIE_PLANNER` names the tick oracle's dump (`load_oracle/probes/planner.py` after
//! 680 updates of MarvinBlue/426048731, in
//! `aoe2de-exe-re/artifacts/playback/planner_426048731_679.txt`), `GENIE_SNAPSHOTS` the directory
//! with the save of that tick (`679.aoe2record`), `GENIE_DAT` the data file. Skips without them.

use genie_core::tile_path::Planner;

/// One planner of the dump: its restriction's values per terrain and the cells of each level.
struct GamePlanner {
    passable: Vec<f32>,
    levels: [Vec<[u8; 2]>; 4],
}

fn game_planners(text: &str) -> Vec<GamePlanner> {
    let mut out: Vec<GamePlanner> = Vec::new();
    for line in text.lines() {
        let mut words = line.split_whitespace();
        match words.next() {
            Some("planner") => out.push(GamePlanner {
                passable: Vec::new(),
                levels: Default::default(),
            }),
            Some("passable") => {
                out.last_mut().unwrap().passable = words.map(|v| v.parse().unwrap()).collect()
            }
            Some(level) if level.starts_with('L') => {
                let l: usize = level[1..].parse().unwrap();
                let row = words.map(|v| {
                    let c = u16::from_str_radix(v, 16).unwrap();
                    [c as u8, (c >> 8) as u8]
                });
                out.last_mut().unwrap().levels[l].extend(row);
            }
            _ => {}
        }
    }
    out
}

/// The planner for the restriction of `unit` (as Gaia's civ has it in the data file), from the save.
fn slice_planner(save: &[u8], dat: &genie_dat::de::DatFile, unit: usize) -> Planner {
    let master = dat.civs[0].units[unit].as_ref().unwrap();
    let restriction = &dat.terrain_restrictions[master.terrain_restriction as usize];
    let terrain = genie_oracle::TerrainTable::from_dat(dat);
    genie_oracle::planner_from_save(
        save,
        &terrain,
        restriction.passable_buildable_dmg_multiplier.clone(),
    )
    .unwrap()
}

fn differences(slice: &Planner, game: &GamePlanner) -> Vec<String> {
    let mut out = Vec::new();
    let (widths, _) = slice.level_sizes();
    for (l, cells) in game.levels.iter().enumerate() {
        let ours = slice.level(l);
        assert_eq!(ours.len(), cells.len(), "level {l}'s size");
        let w = widths[l] as usize;
        for (i, (o, g)) in ours.iter().zip(cells).enumerate() {
            if o != g {
                out.push(format!(
                    "level {l} ({}, {}): slice {o:02x?} game {g:02x?}",
                    i % w,
                    i / w
                ));
            }
        }
    }
    out
}

#[test]
fn the_slice_s_tile_grid_is_the_game_s() {
    let (Some(dump), Some(dir), Some(dat)) = (
        std::env::var_os("GENIE_PLANNER"),
        std::env::var_os("GENIE_SNAPSHOTS"),
        std::env::var_os("GENIE_DAT"),
    ) else {
        return eprintln!("GENIE_PLANNER / GENIE_SNAPSHOTS / GENIE_DAT are not set; skipping");
    };
    let game = game_planners(&std::fs::read_to_string(dump).unwrap());
    let save = std::fs::read(std::path::Path::new(&dir).join("679.aoe2record")).unwrap();
    let dat = genie_dat::de::DatFile::read_from(std::fs::File::open(dat).unwrap()).unwrap();
    // World::load_world makes the planners of Gaia's villager (master 83) and of master 535
    for (unit, game) in [83usize, 535].into_iter().zip(&game) {
        let slice = slice_planner(&save, &dat, unit);
        assert_eq!(
            slice.passable(),
            &game.passable[..],
            "master {unit}'s restriction"
        );
        let d = differences(&slice, game);
        assert!(
            d.is_empty(),
            "master {unit}: {} differences, the first: {:#?}",
            d.len(),
            &d[..d.len().min(20)]
        );
    }
}

/// One logged search of `load_oracle/probes/tilepath.py`: the routine, its request, the game's
/// result byte and path points.
struct GameSearch {
    line: String,
    routine: String,
    request: genie_core::tile_path::Request,
    result: u8,
    points: Vec<[f32; 3]>,
}

/// The numbers in `text`, in order (as f64: the probe prints each f32 exactly).
fn numbers(text: &str) -> Vec<f64> {
    text.split(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == 'e'))
        .filter(|w| !w.is_empty() && w.chars().any(|c| c.is_ascii_digit()))
        .map(|w| w.parse().unwrap())
        .collect()
}

fn game_searches(text: &str) -> Vec<GameSearch> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    for pair in lines.windows(2) {
        let (call, result) = (pair[0], pair[1]);
        if !call.starts_with("tick ") {
            continue;
        }
        let routine = call.split_whitespace().nth(2).unwrap().to_string();
        let part = |from: &str, to: &str| {
            let rest = call.split(from).nth(1).unwrap();
            numbers(rest.split(to).next().unwrap())
        };
        let from = part(" from ", " to ");
        let to = part(" to ", " range ");
        let range = part(" range ", " rects ");
        let flag = call
            .split(" flag ")
            .nth(1)
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap();
        let points = numbers(result.split(" points ").nth(1).unwrap());
        out.push(GameSearch {
            line: call.to_string(),
            routine,
            request: genie_core::tile_path::Request {
                from: [from[0] as f32, from[1] as f32],
                to: [to[0] as f32, to[1] as f32],
                range: range[0] as f32,
                target_range: range[1] as f32,
                closed: Vec::new(),
                full: flag != "0",
            },
            result: numbers(result.split(" length ").next().unwrap())[0] as u8,
            points: points
                .chunks(3)
                .map(|c| [c[0] as f32, c[1] as f32, c[2] as f32])
                .collect(),
        });
    }
    out
}

/// The searches the game ran from update 681 on (`GENIE_TILEPATH`, the probe's log over 170
/// updates from the fork server at 680, in
/// `aoe2de-exe-re/artifacts/playback/tilepath_426048731_681-850.txt`), each searched again on the
/// slice's planner of the 679 save: the same result and the same points.
#[test]
fn the_slice_s_searches_find_the_game_s_paths() {
    let (Some(log), Some(dir), Some(dat)) = (
        std::env::var_os("GENIE_TILEPATH"),
        std::env::var_os("GENIE_SNAPSHOTS"),
        std::env::var_os("GENIE_DAT"),
    ) else {
        return eprintln!("GENIE_TILEPATH / GENIE_SNAPSHOTS / GENIE_DAT are not set; skipping");
    };
    let searches = game_searches(&std::fs::read_to_string(log).unwrap());
    let save = std::fs::read(std::path::Path::new(&dir).join("679.aoe2record")).unwrap();
    let dat = genie_dat::de::DatFile::read_from(std::fs::File::open(dat).unwrap()).unwrap();
    let mut planner = slice_planner(&save, &dat, 83);
    let grid: Vec<Vec<[u8; 2]>> = (0..4).map(|l| planner.level(l).to_vec()).collect();
    let mut checked = 0;
    for s in searches.iter().filter(|s| s.routine != "plan") {
        let mut path = genie_core::tile_path::Path::default();
        let found = match s.routine.as_str() {
            "levels" => planner.search_over_levels(&s.request, &mut path),
            _ => planner.search_on_tiles(&s.request, &mut path),
        };
        assert_eq!(
            (found.code(), &path.points),
            (s.result, &s.points),
            "{}",
            s.line
        );
        let after: Vec<Vec<[u8; 2]>> = (0..4).map(|l| planner.level(l).to_vec()).collect();
        assert!(
            after == grid,
            "the search leaves the grid as it found it: {}",
            s.line
        );
        checked += 1;
    }
    assert_eq!(checked, 16);
}
