//! Read, then write: the DE header gives back its bytes (FD-014: the proof that the reader
//! accounts for every byte). Needs `GENIE_REC_CORPUS`, a directory of recordings; checks the tier-1
//! ones (save 66.6, build 158041) and skips without it.

use binrw::BinWrite;
use binrw::io::Cursor;
use genie_rec::de;
use std::path::{Path, PathBuf};

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

/// The header sections up to the last player, written in read order.
fn write_through_players(h: &de::header::Header) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    h.version.write_le(&mut out).unwrap();
    h.lobby.write_le(&mut out).unwrap();
    h.ai.write_le(&mut out).unwrap();
    h.replay.write_le(&mut out).unwrap();
    h.map.write_le(&mut out).unwrap();
    h.restore_time.write_le(&mut out).unwrap();
    (h.particles.len() as u32 / 27).write_le(&mut out).unwrap();
    h.particles.write_le(&mut out).unwrap();
    h.identifier.write_le(&mut out).unwrap();
    for p in &h.players {
        p.player.write_le(&mut out).unwrap();
    }
    out.into_inner()
}

/// The first offset where `ours` differs from `theirs`, or where the shorter one ends.
fn first_difference(ours: &[u8], theirs: &[u8]) -> Option<usize> {
    let common = ours.iter().zip(theirs).position(|(a, b)| a != b);
    common.or((ours.len() != theirs.len()).then(|| ours.len().min(theirs.len())))
}

#[test]
fn tier_1_headers_write_back_their_bytes() {
    let Ok(dir) = std::env::var("GENIE_REC_CORPUS") else {
        eprintln!("GENIE_REC_CORPUS is not set; skipping the round trip");
        return;
    };
    let mut paths = Vec::new();
    recordings(Path::new(&dir), &mut paths);
    paths.sort();
    let (mut checked, mut differ) = (0, Vec::new());
    for path in &paths {
        let data = std::fs::read(path).unwrap();
        let Ok(chapters) = de::chapters(&data) else {
            continue;
        };
        let Ok(version) = chapters[0].save_version() else {
            continue;
        };
        if (version.hundredths(), version.build) != (6660, 158041) {
            continue;
        }
        let inflated = chapters[0].inflate_header().unwrap();
        let header = de::header::parse(&inflated).unwrap();
        let end = header
            .players
            .last()
            .map_or(header.ends.players_start, |p| p.ends.end);
        if let Some(at) = first_difference(&write_through_players(&header), &inflated[..end]) {
            differ.push(format!("{}: first difference at {at:#x}", path.display()));
        }
        checked += 1;
    }
    eprintln!("{checked} tier-1 headers written back");
    assert!(
        differ.is_empty(),
        "{} headers differ:\n{}",
        differ.len(),
        differ.join("\n")
    );
}

#[test]
fn the_first_difference_is_where_the_bytes_or_the_lengths_part() {
    assert_eq!(first_difference(b"abc", b"abc"), None);
    assert_eq!(first_difference(b"abd", b"abc"), Some(2));
    assert_eq!(first_difference(b"ab", b"abc"), Some(2));
}
