//! Print the map's tile elevations in a window of a recording's header save, both axis orders
//! (rows[y][x] and rows[x][y]): to check positions' z against the terrain.
//!
//! cargo run --release -p genie-oracle --example tile_heights -- REC X0 X1 Y0 Y1
use genie_rec::de;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let data = std::fs::read(&args[1]).expect("the recording");
    let n = |i: usize| args[i].parse::<usize>().expect("a tile coordinate");
    let (x0, x1, y0, y1) = (n(2), n(3), n(4), n(5));
    let chapter = de::chapters(&data).expect("chapters")[0];
    let header = chapter.header().expect("the world save");
    let map = &header.world.base_world.map;
    println!("map {} x {}", map.map_width, map.map_height);
    let at = |r: usize, c: usize| {
        map.rows
            .get(r)
            .and_then(|row| row.tiles.get(c))
            .map(|t| t.t_m_elevation)
    };
    for (name, order) in [("rows[y][x]", false), ("rows[x][y]", true)] {
        println!("{name}: y down, x across {x0}..={x1}");
        for y in y0..=y1 {
            let line: Vec<String> = (x0..=x1)
                .map(|x| {
                    at(if order { x } else { y }, if order { y } else { x })
                        .map_or("-".into(), |e| e.to_string())
                })
                .collect();
            println!("  y {y:3}: {}", line.join(" "));
        }
    }
}
