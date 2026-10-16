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

/// The element count of a named array section.
fn expected_count(section: &str) -> usize {
    FIXTURE
        .lines()
        .filter(|l| !l.starts_with('#'))
        .find_map(|l| {
            let mut f = l.split_whitespace();
            (f.next() == Some(section)).then(|| f.nth(2).unwrap().parse().unwrap())
        })
        .unwrap_or_else(|| panic!("no count for {section} in the fixture"))
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

    assert_eq!(header.players.len(), 3);
    // The boundaries inside a player section are checked against the game's own reads
    // (tests/de_oracle.rs); against ImHex, the section starts, ends and counts.
    for (i, p) in header.players.iter().enumerate() {
        let name = |part: &str| format!("player{i}{part}");
        assert_eq!(p.ends.start, expected_start(&name("")), "player {i}");
        assert_eq!(
            p.objects.masters.as_ref().map_or(0, Vec::len),
            expected_count(&name(".initial_objects")),
            "player {i}"
        );
        assert_eq!(
            p.live_objects().count(),
            expected_count(&name(".objects")),
            "player {i}"
        );
    }
    // Every section, Gaia's included, is read to its end, and the next one starts right there. The
    // pattern skips Gaia's tail by searching for player 1; ImHex's offset for player 1 is the check.
    assert_eq!(header.players[0].ends.end, expected_start("player1"));
    for (i, p) in header.players.iter().enumerate().skip(1) {
        assert_eq!(
            (
                p.sleeping_objects().count(),
                p.objects.doppleganger_objects.objects().count()
            ),
            (0, 0),
            "player {i}"
        );
        assert_eq!(p.ends.end, expected(&format!("player{i}")), "player {i}");
    }
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

/// How far the corpus parses. Tier 1 (save 66.6, build 158041, the analysed binary) is strict: every
/// recording must parse. The reader is sequential, so each player section must end where the next
/// one starts. Other versions are only reported.
#[test]
fn every_corpus_header_parses() {
    let Some(dir) = corpus() else {
        return eprintln!("GENIE_REC_CORPUS is not set; skipping");
    };
    let mut paths = Vec::new();
    recordings(&dir, &mut paths);
    let mut tier1 = std::collections::BTreeMap::<String, usize>::new();
    let mut others = std::collections::BTreeMap::<String, usize>::new();
    let mut tier1_failures = Vec::new();
    for path in &paths {
        let data = std::fs::read(path).unwrap();
        let chapter = de::chapters(&data).unwrap()[0];
        let version = chapter.save_version().unwrap();
        let is_tier1 = (version.hundredths(), version.build) == (6660, 158041);
        let outcome = match de::header::parse(&chapter.inflate_header().unwrap()) {
            Ok(_) => "parsed".to_string(),
            Err(e) => {
                if is_tier1 {
                    tier1_failures.push(format!(
                        "{}: {e}",
                        path.strip_prefix(&dir).unwrap().display()
                    ));
                }
                match e {
                    de::Error::Unsupported { what, .. } => format!("unsupported: {what}"),
                    _ => "failed".to_string(),
                }
            }
        };
        *(if is_tier1 { &mut tier1 } else { &mut others })
            .entry(outcome)
            .or_default() += 1;
    }
    eprintln!("tier 1: {tier1:?}\nother versions: {others:?}");
    assert!(tier1_failures.is_empty(), "{tier1_failures:#?}");
    assert_eq!(tier1.get("parsed"), Some(&137));
}
