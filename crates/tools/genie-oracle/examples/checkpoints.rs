//! A recording's run from its header, nothing forced, against the recording's own checkpoints
//! (every 500 updates): per checkpoint, each player's words of the 11-word block that differ, as
//! `(word, engine, game)`, or `ok`.
//!
//! `cargo run --release -p genie-oracle --example checkpoints -- REC DAT UPDATES`

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let [_, rec, dat, updates] = &args[..] else {
        return Err("usage: checkpoints REC DAT UPDATES".into());
    };
    let recording = std::fs::read(rec)?;
    let masters = genie_oracle::MasterTable::read(dat.as_ref())?;
    print!(
        "{}",
        genie_oracle::views::checkpoints(&recording, &masters, updates.parse()?)?
    );
    Ok(())
}
