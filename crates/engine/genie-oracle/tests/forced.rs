//! The forced replay (FD-008, CONTINUE.md): genie-core from the game's own save after every
//! update of a window, one step each, against the game's next save, for every player. Each step
//! starts from the game's state, so nothing the slice gets wrong (a freeze, a wrong draw) carries
//! into the next: the report lists everything the slice does not model in the window, ranked,
//! instead of the first freeze. Dev only: the per-tick saves and dumps come from the tick oracle
//! (`load_oracle/probes/per_tick.py`), which genie-core never reads. Not pinned, and opt-in: it
//! runs with `GENIE_FORCED` set (and `GENIE_SNAPSHOTS`, `GENIE_TICK_REC`, `GENIE_DAT`).
//!
//! Two modes (`GENIE_FORCED_MODE`): `run` (the default) plays the window from the game's save at
//! its start (or segments of `GENIE_FORCED_SEGMENT` updates, in parallel, each from its own
//! save), every update compared and then forced where the slice did not model it or got it wrong
//! (`genie_oracle::forced_run`: the frame seed, the frozen and the differing objects, a stopped
//! player, the sight), so the objects it gets right keep their own unsaved state; `restart`
//! starts every update from the game's save (`genie_oracle::forced_ticks`, `GENIE_FORCED_TICKS`
//! updates each, default 1), which also checks the save's import, but loses unsaved state.
//!
//! `GENIE_FORCED_FIRST`, `GENIE_FORCED_LAST` (the window's first and last start, default 680 and
//! 1500) and `GENIE_FORCED_OUT` (the Markdown report, default `target/tmp/forced_report.md`).

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};

use genie_oracle::TickReport;

fn env_or(name: &str, default: usize) -> usize {
    std::env::var(name).map_or(default, |v| v.parse().unwrap())
}

/// A save's mesh dump: `<tick>.mesh` or `<tick>.mesh.gz`.
fn mesh(dir: &Path, tick: usize) -> Option<String> {
    if let Ok(text) = std::fs::read_to_string(dir.join(format!("{tick}.mesh"))) {
        return Some(text);
    }
    let raw = std::fs::read(dir.join(format!("{tick}.mesh.gz"))).ok()?;
    let mut text = String::new();
    flate2::read::GzDecoder::new(&raw[..])
        .read_to_string(&mut text)
        .unwrap();
    Some(text)
}

/// A segment of the continuous mode from the save after `start`, `ticks` updates.
fn segment(
    dir: &Path,
    rec: &[u8],
    masters: &genie_oracle::MasterTable,
    start: usize,
    ticks: usize,
) -> Vec<TickReport> {
    let Ok(save) = std::fs::read(dir.join(format!("{start}.aoe2record"))) else {
        return Vec::new();
    };
    let mesh = mesh(dir, start);
    let registry = std::fs::read_to_string(dir.join(format!("{start}.registry"))).ok();
    let after = |t: usize| std::fs::read(dir.join(format!("{t}.aoe2record"))).ok();
    genie_oracle::forced_run(
        &save,
        rec,
        start,
        ticks,
        Some(masters),
        genie_oracle::Dumps {
            mesh: mesh.as_deref(),
            registry: registry.as_deref(),
        },
        &after,
    )
    .unwrap()
}

/// One step from the save after `start`; None when the window lacks its files.
fn step(
    dir: &Path,
    rec: &[u8],
    masters: &genie_oracle::MasterTable,
    start: usize,
    ticks: usize,
) -> Option<TickReport> {
    let save = std::fs::read(dir.join(format!("{start}.aoe2record"))).ok()?;
    let after = std::fs::read(dir.join(format!("{}.aoe2record", start + ticks))).ok()?;
    let mesh = mesh(dir, start);
    let registry = std::fs::read_to_string(dir.join(format!("{start}.registry"))).ok();
    Some(
        genie_oracle::forced_ticks(
            &save,
            &after,
            rec,
            start,
            ticks,
            Some(masters),
            genie_oracle::Dumps {
                mesh: mesh.as_deref(),
                registry: registry.as_deref(),
            },
        )
        .unwrap(),
    )
}

/// What a difference is about, without its object, tile or values: `9183: position slice …` is
/// `position`, `player 1 tile (91, 55) count 2, the game 3` is `sight: tile count`.
fn field(d: &str) -> String {
    if let Some(rest) = d.strip_prefix("player ") {
        let words: Vec<&str> = rest
            .split_whitespace()
            .skip(1)
            .filter(|w| !w.starts_with('(') && !w.ends_with(')') && w.parse::<f64>().is_err())
            .take_while(|w| *w != "the")
            .collect();
        return format!("sight: {}", words.join(" ").trim_end_matches(','));
    }
    let after = d.split_once(": ").map_or(d, |(_, r)| r);
    if after.starts_with("only in the game") {
        return "only in the game".to_string();
    }
    after
        .split_once(" slice ")
        .map_or(after, |(f, _)| f)
        .to_string()
}

#[derive(Default)]
struct Tally {
    count: usize,
    first: usize,
    /// The first step without any update the slice did not model: there the difference is the
    /// object's own, not a frozen object's missing draws or moves earlier in the update.
    first_clean: Option<usize>,
    players: BTreeSet<u32>,
    objects: BTreeSet<i32>,
}

impl Tally {
    fn add(&mut self, start: usize, player: u32, object: Option<i32>, clean: bool) {
        if self.count == 0 || start < self.first {
            self.first = start;
        }
        if clean && self.first_clean.is_none_or(|f| start < f) {
            self.first_clean = Some(start);
        }
        self.count += 1;
        self.players.insert(player);
        if let Some(o) = object {
            self.objects.insert(o);
        }
    }

    fn row(&self, what: &str) -> String {
        let objects: Vec<String> = self.objects.iter().take(6).map(|o| o.to_string()).collect();
        let more = if 6 < self.objects.len() { " …" } else { "" };
        let clean = self.first_clean.map_or("-".to_string(), |f| f.to_string());
        format!(
            "| {what} | {} | {} | {clean} | {:?} | {}{more} |",
            self.count,
            self.first,
            self.players,
            objects.join(", ")
        )
    }
}

fn ranked(map: &BTreeMap<String, Tally>) -> Vec<(&String, &Tally)> {
    let mut v: Vec<_> = map.iter().collect();
    v.sort_by(|a, b| a.1.first.cmp(&b.1.first).then(b.1.count.cmp(&a.1.count)));
    v
}

#[test]
fn the_forced_replay_report() {
    if std::env::var_os("GENIE_FORCED").is_none() {
        return eprintln!("GENIE_FORCED is not set; skipping the forced replay");
    }
    let (Some(dir), Some(rec), Some(masters)) = (
        std::env::var_os("GENIE_SNAPSHOTS").map(PathBuf::from),
        std::env::var_os("GENIE_TICK_REC").map(|p| std::fs::read(p).unwrap()),
        common::masters(),
    ) else {
        return eprintln!("GENIE_SNAPSHOTS / GENIE_TICK_REC / GENIE_DAT are not set; skipping");
    };
    let first = env_or("GENIE_FORCED_FIRST", 680);
    let last = env_or("GENIE_FORCED_LAST", 1500);
    let ticks = env_or("GENIE_FORCED_TICKS", 1);
    let restart = std::env::var("GENIE_FORCED_MODE").is_ok_and(|m| m == "restart");
    let length = env_or("GENIE_FORCED_SEGMENT", usize::MAX).max(1);
    let out = std::env::var_os("GENIE_FORCED_OUT").map_or_else(
        || PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("forced_report.md"),
        PathBuf::from,
    );
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let mut reports: Vec<TickReport> = if restart {
        let starts: Vec<usize> = (first..=last).collect();
        let chunk = starts.len().div_ceil(threads).max(1);
        std::thread::scope(|s| {
            let handles: Vec<_> = starts
                .chunks(chunk)
                .map(|part| {
                    let (dir, rec, masters) = (&dir, &rec, &masters);
                    s.spawn(move || {
                        part.iter()
                            .filter_map(|&t| step(dir, rec, masters, t, ticks))
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            handles
                .into_iter()
                .flat_map(|h| h.join().unwrap())
                .collect()
        })
    } else {
        let segments: Vec<(usize, usize)> = (first..=last)
            .step_by(length)
            .map(|s| (s, length.min(last + 1 - s)))
            .collect();
        std::thread::scope(|s| {
            let handles: Vec<_> = segments
                .iter()
                .map(|&(start, n)| {
                    let (dir, rec, masters) = (&dir, &rec, &masters);
                    s.spawn(move || segment(dir, rec, masters, start, n))
                })
                .collect();
            handles
                .into_iter()
                .flat_map(|h| h.join().unwrap())
                .collect()
        })
    };
    reports.sort_by_key(|r| r.start);
    let mut unsupported: BTreeMap<String, Tally> = BTreeMap::new();
    let mut differences: BTreeMap<String, Tally> = BTreeMap::new();
    // the slice's update numbers (as its freezes name them): the step from save file S ends in
    // the save file S + ticks, which holds the state after update S + ticks + 1
    let update = |r: &TickReport| r.start + r.ticks + 1;
    for r in &reports {
        for &(player, object, _, why) in &r.unsupported {
            unsupported
                .entry(why.0.to_string())
                .or_default()
                .add(update(r), player, object, true);
        }
        let clean = r.unsupported.is_empty();
        for (player, d) in &r.differences {
            let object = d.split(':').next().and_then(|o| o.parse::<i32>().ok());
            differences
                .entry(field(d))
                .or_default()
                .add(update(r), *player, object, clean);
        }
    }
    let clean = reports
        .iter()
        .filter(|r| r.unsupported.is_empty() && r.differences.is_empty())
        .count();
    let mut md = format!(
        "# Forced replay ({}): from the save files {first}..{last}\n\n\
         {} updates run, {clean} without a difference or an unmodelled update. Updates are the \
         slice's numbers (its freezes'): the step from save file S plays update S + 2.\n\n\
         ## Not modelled, by first update\n\n| why | count | first | - | players | objects |\n\
         | --- | --- | --- | --- | --- | --- |\n",
        if restart {
            format!("a restart from every save, {ticks} update(s) each")
        } else if length == usize::MAX {
            "one run, forced after each update".to_string()
        } else {
            format!("segments of {length} updates, forced after each update")
        },
        reports.len()
    );
    for (why, t) in ranked(&unsupported) {
        md += &t.row(why);
        md.push('\n');
    }
    md += "\n## Differences of objects that ran, by field and first update\n\n\
           The second update column is the first update without anything unmodelled (where the \
           difference is the object's own, not an earlier frozen object's missing draws).\n\n\
           | field | count | first | first without a freeze | players | objects |\n\
           | --- | --- | --- | --- | --- | --- |\n";
    for (f, t) in ranked(&differences) {
        md += &t.row(f);
        md.push('\n');
    }
    md += "\n## The first updates with differences\n\n";
    for r in reports
        .iter()
        .filter(|r| !r.differences.is_empty())
        .take(40)
    {
        md += &format!("- update {} (from the save file {}):\n", update(r), r.start);
        for (player, d) in r.differences.iter().take(8) {
            let d: String = d.chars().take(300).collect();
            md += &format!("  - player {player}: {d}\n");
        }
    }
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&out, &md).unwrap();
    eprintln!(
        "forced replay: {} steps, {clean} clean, {} unmodelled reasons, {} differing fields; {}",
        reports.len(),
        unsupported.len(),
        differences.len(),
        out.display()
    );
}
