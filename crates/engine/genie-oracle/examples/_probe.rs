use genie_rec::de;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let data = std::fs::read(&a[1]).unwrap();
    let chapter = de::chapters(&data).unwrap()[0];
    let header = de::header::parse(&chapter.inflate_header().unwrap()).unwrap();
    let tiles = [
        (70, 22),
        (72, 24),
        (43, 26),
        (113, 46),
        (114, 46),
        (114, 48),
        (51, 48),
    ];
    for (pi, section) in header.players().iter().enumerate() {
        let lists = [
            (
                "live",
                section.objects().objects.objects().collect::<Vec<_>>(),
            ),
            (
                "sleep",
                section.objects().sleeping_objects.objects().collect(),
            ),
            (
                "doppel",
                section.objects().doppleganger_objects.objects().collect(),
            ),
        ];
        for (name, list) in lists {
            for o in list {
                let Some(s) = o.static_part() else { continue };
                if tiles.contains(&(s.t_x as i32, s.t_y as i32)) {
                    let m = section
                        .objects()
                        .master(s.master_obj)
                        .and_then(|m| m.static_part());
                    println!(
                        "p{pi} {name} id {} master {} at ({}, {}) state {} kind {:?} radius {:?} inside {}",
                        s.id,
                        s.master_obj,
                        s.t_x,
                        s.t_y,
                        s.object_state,
                        m.map(|m| m.obstruction_type),
                        m.map(|m| (m.radius_x, m.radius_y, m.radius_z)),
                        s.inside_obj
                    );
                }
            }
        }
    }
}
