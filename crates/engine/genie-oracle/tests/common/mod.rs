//! Corpus access shared by the oracle's integration tests. Each test crate uses a part of it.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

/// The corpus directory from `GENIE_REC_CORPUS`; the tests skip without it.
pub fn corpus() -> Option<PathBuf> {
    std::env::var_os("GENIE_REC_CORPUS").map(PathBuf::from)
}

/// The tier-1 recordings (save 66.6, build 158041) under `dir`, sorted, with their bytes.
pub fn tier1_recordings(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut paths = Vec::new();
    collect(dir, &mut paths);
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let data = std::fs::read(&path).unwrap();
            (path, data)
        })
        .filter(|(_, data)| {
            let version = genie_rec::de::chapters(data).unwrap()[0]
                .save_version()
                .unwrap();
            (version.hundredths(), version.build) == (6660, 158041)
        })
        .collect()
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap().map(Result::unwrap) {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, out);
        } else if path.extension().is_some_and(|e| e == "aoe2record") {
            out.push(path);
        }
    }
}

/// The lobby's game mode 3 (GameOptions' `mGameMode`), a scenario.
pub fn is_scenario(recording: &[u8]) -> bool {
    let chapter = genie_rec::de::chapters(recording).unwrap()[0];
    let header = chapter.header().unwrap();
    header.file_header.game_options_read.game_options.game_mode == 3
}

/// The master table of the game data file `GENIE_DAT` (an `empires2_x2_p1.dat`), with the
/// drop-site table beside it (`dropsites.json`) if there is one; the slice's tests skip without
/// it, as every start of game has objects that die.
pub fn masters() -> Option<genie_oracle::MasterTable> {
    let path = PathBuf::from(std::env::var_os("GENIE_DAT")?);
    let dat = genie_dat::de::DatFile::read_from(std::fs::File::open(&path).unwrap()).unwrap();
    let mut table = genie_oracle::MasterTable::from_dat(&dat);
    if let Ok(json) = std::fs::read_to_string(path.with_file_name("dropsites.json")) {
        table.drop_sites = genie_oracle::drop_sites(&json).unwrap();
    }
    Some(table)
}
