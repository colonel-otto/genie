//! What the players see, against the game: `GENIE_SNAPSHOTS` names the directory of the tick
//! oracle's saves of MarvinBlue/426048731 with the map tiles' building links after some updates
//! (`<tick>.tilelinks`: `load_oracle/probes/tile_links.py`, which the save does not hold),
//! `GENIE_TICK_REC` the recording, `GENIE_DAT` the build's `.dat`. Skips without them.

// the dumps' numbers fit the game's widths; a test runs the whole comparison
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::too_many_lines
)]

mod common;

use genie_core::visibility::Visibility;
use std::path::{Path, PathBuf};

/// The linked tiles in the probe's format (`x y flags dir`), the flags cut to the link bits.
fn links(sight: &Visibility) -> Vec<(i32, i32, u8, u8)> {
    let mut out = Vec::new();
    for y in 0..sight.height {
        for x in 0..sight.width {
            let (bits, byte) = sight.link(x, y);
            if bits & 0x60 != 0 {
                out.push((x, y, bits & 0x60, byte));
            }
        }
    }
    out
}

fn dumped_links(dir: &Path, tick: usize) -> Vec<(i32, i32, u8, u8)> {
    let dump = std::fs::read_to_string(dir.join(format!("{tick}.tilelinks"))).unwrap();
    dump.lines()
        .map(|l| {
            let v: Vec<i32> = l.split_whitespace().map(|w| w.parse().unwrap()).collect();
            (v[0], v[1], v[2] as u8 & 0x60, v[3] as u8)
        })
        .collect()
}

#[test]
fn the_buildings_tile_links_from_a_save_are_the_game_s() {
    let Some(dir) = std::env::var_os("GENIE_SNAPSHOTS").map(PathBuf::from) else {
        return eprintln!("GENIE_SNAPSHOTS is not set; skipping");
    };
    let save = std::fs::read(dir.join("680.aoe2record")).unwrap();
    let spatial = genie_oracle::spatial_from_save(&save).unwrap();
    let slice = links(spatial.sight.as_ref().unwrap());
    let game = dumped_links(&dir, 680);
    assert_eq!(slice.len(), 48);
    assert_eq!(slice, game);
}

#[test]
fn a_foundation_s_tiles_link_when_its_construction_starts() {
    // the house 10028, placed by the BUILD of update 680 and made desolid (its tiles unlinked),
    // starts in 731: its footprint links to its anchor again
    let Some(dir) = std::env::var_os("GENIE_SNAPSHOTS").map(PathBuf::from) else {
        return eprintln!("GENIE_SNAPSHOTS is not set; skipping");
    };
    let (Some(rec), Some(masters)) = (std::env::var_os("GENIE_TICK_REC"), common::masters()) else {
        return eprintln!("GENIE_TICK_REC / GENIE_DAT are not set; skipping");
    };
    let rec = std::fs::read(rec).unwrap();
    let save = std::fs::read(dir.join("680.aoe2record")).unwrap();
    let mesh = std::fs::read_to_string(dir.join("680.mesh")).unwrap();
    // the TC's idle checks search the objects in sight (690)
    let registry = std::fs::read_to_string(dir.join("680.registry")).unwrap();
    let dumps = genie_oracle::Dumps {
        mesh: Some(&mesh),
        registry: Some(&registry),
    };
    let run =
        genie_oracle::objects_from_save(&save, &rec, 680, 60, 2, Some(&masters), dumps).unwrap();
    assert_eq!(run.unsupported, None);
    let slice = links(run.visibility.as_ref().unwrap());
    assert_eq!(slice.len(), 52);
    assert_eq!(slice, dumped_links(&dir, 740));
}

#[test]
fn a_save_s_sight_read_as_the_slice_s_start_is_the_save() {
    let Some(dir) = std::env::var_os("GENIE_SNAPSHOTS").map(PathBuf::from) else {
        return eprintln!("GENIE_SNAPSHOTS is not set; skipping");
    };
    let save = std::fs::read(dir.join("680.aoe2record")).unwrap();
    let spatial = genie_oracle::spatial_from_save(&save).unwrap();
    let game = genie_oracle::saved_visibility(&save).unwrap();
    for player in 0..3 {
        let found =
            genie_oracle::visibility_differences(spatial.sight.as_ref().unwrap(), &game, player);
        assert_eq!(found, Vec::<String>::new());
    }
    // the town centre's circle of 8 counts on its player's map
    let tc = spatial.sight.as_ref().unwrap().seer(9156).unwrap();
    assert_eq!((tc.player, tc.master, tc.los), (2, 109, 8.0));
}

/// Player `p`'s registry lists in the dump's format without the records of the objects
/// `frozen`, and the masks and slots of the objects `ids`.
fn registry_lines(
    r: &genie_core::registry::Registry,
    p: usize,
    ids: &[i32],
    frozen: &[i32],
) -> Vec<String> {
    let mut out = Vec::new();
    for (c, list) in r.lists[p].iter().enumerate() {
        let list: Vec<_> = list.iter().filter(|x| !frozen.contains(&x.id)).collect();
        if list.is_empty() {
            continue;
        }
        out.push(format!("list {p} {c} {}", list.len()));
        for x in list {
            out.push(format!(
                "rec {} {:08x} {} {:08x} {:08x} {:08x} {:08x}",
                x.id,
                x.distance.to_bits(),
                x.owner,
                x.x.to_bits(),
                x.y.to_bits(),
                x.rx.to_bits(),
                x.ry.to_bits()
            ));
        }
    }
    for &id in ids {
        let e = r.entry(id).copied().unwrap_or_default();
        let slots: Vec<String> = e
            .slots
            .iter()
            .map(|s| s.map_or("-1".to_string(), |i| i.to_string()))
            .collect();
        out.push(format!("mask {id} {:x} {}", e.mask, slots.join(" ")));
    }
    out
}

#[test]
fn player_2_s_registry_from_a_save_matches_the_game_s_through_1180() {
    // the slice from the save after 680 with the game's registry then, through the save after
    // 1180: player 2's lists (its own objects and the livestock it sees, less the objects the
    // slice stopped: Gaia's bird 9926, frozen before 880, flew through its sight) and its objects'
    // masks
    let Some(dir) = std::env::var_os("GENIE_SNAPSHOTS").map(PathBuf::from) else {
        return eprintln!("GENIE_SNAPSHOTS is not set; skipping");
    };
    let (Some(rec), Some(masters)) = (std::env::var_os("GENIE_TICK_REC"), common::masters()) else {
        return eprintln!("GENIE_TICK_REC / GENIE_DAT are not set; skipping");
    };
    let rec = std::fs::read(rec).unwrap();
    let save = std::fs::read(dir.join("680.aoe2record")).unwrap();
    let mesh = std::fs::read_to_string(dir.join("680.mesh")).unwrap();
    let start = std::fs::read_to_string(dir.join("680.registry")).unwrap();
    let dumps = genie_oracle::Dumps {
        mesh: Some(&mesh),
        registry: Some(&start),
    };
    let run =
        genie_oracle::objects_from_save(&save, &rec, 680, 500, 2, Some(&masters), dumps).unwrap();
    assert_eq!(run.unsupported, None);
    let game = genie_oracle::registry_from_dump(
        &std::fs::read_to_string(dir.join("1180.registry")).unwrap(),
    )
    .unwrap();
    let ids: Vec<i32> = run.objects.iter().map(|o| o.id).collect();
    let slice = run.visibility.as_ref().unwrap().registry.as_ref().unwrap();
    assert_eq!(
        registry_lines(slice, 2, &ids, &run.frozen),
        registry_lines(&game, 2, &ids, &run.frozen)
    );
}
