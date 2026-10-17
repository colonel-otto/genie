fn class(f: u8) -> u32 {
    let f = f as u32;
    if f == 0 {
        0
    } else if f & 7 != 0 {
        1
    } else if f & 0x18 != 0 {
        2
    } else {
        (f & 0xff) >> 3 & 4
    }
}
fn main() {
    let f = std::fs::File::open(std::env::var("GENIE_DAT").unwrap()).unwrap();
    let dat = genie_dat::de::DatFile::read_from(f).unwrap();
    let terrains = &dat.terrain_block.terrains;
    println!(
        "{} restrictions, {} terrains (used {})",
        dat.terrain_restrictions.len(),
        terrains.len(),
        dat.terrains_used
    );
    let mut out = vec![];
    for r in &dat.terrain_restrictions {
        let mut m = 0;
        for (t, v) in r.passable_buildable_dmg_multiplier.iter().enumerate() {
            if *v > 0.0 {
                if let Some(tt) = terrains.get(t) {
                    m |= class(tt.is_water);
                }
            }
        }
        out.push(format!("{m:x}"));
    }
    println!("{}", out.join(" "));
    println!(
        "flags {}",
        terrains
            .iter()
            .take(120)
            .enumerate()
            .filter(|(_, t)| t.is_water != 0)
            .map(|(i, t)| format!("{i}:{:x}", t.is_water))
            .collect::<Vec<_>>()
            .join(" ")
    );
}
