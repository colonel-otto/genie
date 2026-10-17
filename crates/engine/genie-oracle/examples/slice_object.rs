//! The slice's row of one object after TICKS updates from the save after START (args: DIR START
//! TICKS PLAYER ID; env GENIE_TICK_REC, GENIE_DAT): every field the oracle compares, for reading a
//! difference in full.
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let dir = std::path::PathBuf::from(&a[1]);
    let start: usize = a[2].parse().unwrap();
    let ticks: usize = a[3].parse().unwrap();
    let player: u32 = a[4].parse().unwrap();
    let id: i32 = a[5].parse().unwrap();
    let rec = std::fs::read(std::env::var_os("GENIE_TICK_REC").unwrap()).unwrap();
    let path = std::path::PathBuf::from(std::env::var_os("GENIE_DAT").unwrap());
    let dat = genie_dat::de::DatFile::read_from(std::fs::File::open(&path).unwrap()).unwrap();
    let mut masters = genie_oracle::MasterTable::from_dat(&dat);
    if let Ok(json) = std::fs::read_to_string(path.with_file_name("dropsites.json")) {
        masters.drop_sites = genie_oracle::drop_sites(&json).unwrap();
    }
    let save = std::fs::read(dir.join(format!("{start}.aoe2record"))).unwrap();
    let mesh = std::fs::read_to_string(dir.join(format!("{start}.mesh"))).ok();
    let registry = std::fs::read_to_string(dir.join(format!("{start}.registry"))).ok();
    let run = genie_oracle::objects_from_save(
        &save,
        &rec,
        start,
        ticks,
        player,
        Some(&masters),
        genie_oracle::Dumps {
            mesh: mesh.as_deref(),
            registry: registry.as_deref(),
        },
    )
    .unwrap();
    for o in run.objects.iter().filter(|o| o.id == id) {
        println!("{o:#?}");
    }
}
