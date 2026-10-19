//! The run from a recording's header, nothing forced, against the recording's own checkpoints
//! (every 500 updates): per checkpoint, each player's words of the 11-word block that differ, as
//! `(word, slice, game)`, or `ok`. The checkpoint after N syncs holds the state after update N - 1.
//!
//! `GENIE_DAT=… cargo run --release -p genie-oracle --example checkpoints -- REC UPDATES`

use std::fmt::Write;

use genie_rec::de::{self, Op};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let [_, path, updates] = &args[..] else {
        eprintln!("usage: checkpoints REC UPDATES (GENIE_DAT names the data file)");
        std::process::exit(2);
    };
    let updates: usize = updates.parse().expect("a number of updates");
    let rec = std::fs::read(path).expect("the recording");
    let dat_path = std::path::PathBuf::from(std::env::var_os("GENIE_DAT").expect("GENIE_DAT"));
    let dat = genie_dat::de::DatFile::read_from(std::fs::File::open(&dat_path).expect("the data"))
        .expect("a data file");
    let mut masters = genie_oracle::MasterTable::from_dat(&dat);
    if let Ok(json) = std::fs::read_to_string(dat_path.with_file_name("dropsites.json")) {
        masters.drop_sites = genie_oracle::drop_sites(&json).expect("the drop sites");
    }
    let chapter = de::chapters(&rec).expect("chapters")[0];
    let players = chapter.header().expect("the world save").players().len();
    let ids: Vec<u32> = (0..players)
        .map(|p| u32::try_from(p).expect("a player"))
        .collect();
    let blocks =
        genie_oracle::blocks_per_tick(&rec, &ids, updates, Some(&masters)).expect("the run");
    let mut tick = 0usize;
    for op in chapter.ops() {
        match op.expect("an operation") {
            Op::Sync { .. } => tick += 1,
            Op::Checkpoint { checkpoint, .. } => {
                if tick > blocks.len() {
                    break;
                }
                let row = &blocks[tick - 1];
                let mut line = format!("checkpoint at {tick}:");
                for lane in checkpoint
                    .lanes
                    .iter()
                    .filter(|l| l.iter().any(|&w| w != 0))
                {
                    let p = lane[8] as usize;
                    let differ: Vec<(usize, u32, u32)> = (0..11)
                        .filter(|&w| row[p][w] != lane[w])
                        .map(|w| (w, row[p][w], lane[w]))
                        .collect();
                    let _ = if differ.is_empty() {
                        write!(line, " p{p} ok")
                    } else {
                        write!(line, " p{p} {differ:?}")
                    };
                }
                println!("{line}");
                tick += 1;
            }
            _ => {}
        }
    }
}
