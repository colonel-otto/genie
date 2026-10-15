//! The DE header reader against ImHex's offsets for aoe2rec's pattern on one recording, and
//! against every tier-1 recording of the corpus. Needs `GENIE_REC_CORPUS`; skips without it.

use genie_rec::de;
use std::path::{Path, PathBuf};

const FIXTURE: &str = include_str!("fixtures/marvinblue-426048731-header-offsets.txt");

fn corpus() -> Option<PathBuf> {
    std::env::var_os("GENIE_REC_CORPUS").map(PathBuf::from)
}

/// The expected end offset of a named section.
fn expected(section: &str) -> usize {
    FIXTURE
        .lines()
        .filter(|l| !l.starts_with('#'))
        .find_map(|l| {
            let mut f = l.split_whitespace();
            (f.next() == Some(section)).then(|| {
                usize::from_str_radix(f.nth(1).unwrap().trim_start_matches("0x"), 16).unwrap()
            })
        })
        .unwrap_or_else(|| panic!("no {section} in the fixture"))
}

/// The expected start offset of a named section.
fn expected_start(section: &str) -> usize {
    FIXTURE
        .lines()
        .filter(|l| !l.starts_with('#'))
        .find_map(|l| {
            let mut f = l.split_whitespace();
            (f.next() == Some(section)).then(|| {
                usize::from_str_radix(f.next().unwrap().trim_start_matches("0x"), 16).unwrap()
            })
        })
        .unwrap_or_else(|| panic!("no {section} in the fixture"))
}

fn header_of(path: &Path) -> Vec<u8> {
    let data = std::fs::read(path).unwrap();
    de::chapters(&data).unwrap()[0].inflate_header().unwrap()
}

#[test]
fn marvinblue_sections_end_where_imhex_says() {
    let Some(dir) = corpus() else {
        return eprintln!("GENIE_REC_CORPUS is not set; skipping");
    };
    let header =
        de::header::parse(&header_of(&dir.join("MarvinBlue/426048731.aoe2record"))).unwrap();
    assert_eq!(
        (
            header.version.major,
            header.version.build,
            header.version.hundredths()
        ),
        (66, 158041, 6660)
    );
    let ends = header.ends;
    assert_eq!(ends.lobby, expected("lobby_settings"));
    assert_eq!(ends.ai, expected("ai_config"));
    assert_eq!(ends.replay, expected("replay"));
    assert_eq!(ends.map, expected("map_info"));
    assert_eq!(ends.players_start, expected_start("players"));
}

fn recordings(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap().map(Result::unwrap) {
        let path = entry.path();
        if path.is_dir() {
            recordings(&path, out);
        } else if path.extension().is_some_and(|e| e == "aoe2record") {
            out.push(path);
        }
    }
}

#[test]
fn every_corpus_header_parses() {
    let Some(dir) = corpus() else {
        return eprintln!("GENIE_REC_CORPUS is not set; skipping");
    };
    let mut paths = Vec::new();
    recordings(&dir, &mut paths);
    let (mut failures, mut with_ai) = (Vec::new(), 0);
    for path in &paths {
        let header = header_of(path);
        match de::header::parse(&header) {
            Ok(_) => {}
            // the known gap: aoe2rec's AI layout does not fit games with AI scripts
            Err(de::Error::Unsupported {
                what: "AI data (has_ai = 1)",
                ..
            }) => with_ai += 1,
            Err(e) => failures.push(format!(
                "{}: {e}",
                path.strip_prefix(&dir).unwrap().display()
            )),
        }
    }
    eprintln!(
        "{} recordings, {with_ai} with unsupported AI data, {} failed",
        paths.len(),
        failures.len()
    );
    assert!(failures.is_empty(), "{failures:#?}");
}
