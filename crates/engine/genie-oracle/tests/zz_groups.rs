#[test]
fn groups() {
    use genie_rec::de;
    let data =
        std::fs::read("/work/unobfuscated/recorded-games/MarvinBlue/426048731.aoe2record").unwrap();
    let h = de::header::parse(&de::chapters(&data).unwrap()[0].inflate_header().unwrap()).unwrap();
    for (pid, p) in h.players.iter().enumerate() {
        let mut g = std::collections::BTreeMap::<i32, Vec<(u16, u8)>>::new();
        for o in &p.objects {
            g.entry(o.static_part().group_id)
                .or_default()
                .push((o.static_part().main_id, o.kind));
        }
        let grouped: Vec<_> = g
            .iter()
            .filter(|(k, _)| **k != -1)
            .map(|(k, v)| (k, v.len(), v[0]))
            .collect();
        eprintln!(
            "player {pid}: ungrouped {} grouped {:?}",
            g.get(&-1).map_or(0, |v| v.len()),
            grouped
        );
    }
}
