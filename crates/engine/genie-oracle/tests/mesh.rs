//! The short pather's shape mesh, against the game's own: `GENIE_MESH` names the tick oracle's
//! dump after 680 updates of MarvinBlue/426048731 (`load_oracle/probes/mesh.py`, in
//! `aoe2de-exe-re/artifacts/playback/mesh_426048731_679.txt`), `GENIE_MESH_OPS` a run of the
//! game's own mesh routines on that mesh (`load_oracle/probes/meshops.py`: removes, adds and
//! replaces, then the mesh after them). Skips without them.

use genie_core::mesh::Mesh;

/// The mesh in the probe's format, for a readable difference.
fn dump(m: &Mesh) -> Vec<String> {
    let mut out = vec![format!(
        "mesh width {} shapes {} free {:#x}",
        m.width,
        m.shapes.len(),
        m.free
    )];
    for (i, r) in m.shapes.iter().enumerate() {
        out.push(format!("shape {i} {} {} {} {}", r[0], r[1], r[2], r[3]));
        for (s, side) in m.sides[i].iter().enumerate() {
            for c in side {
                out.push(format!(
                    "  side {s} {:x} {:x} {:x} {:x} {:x} {:x} {:x}",
                    c.other, c.side, c.at[0] as u32, c.at[1] as u32, c.along as u32, c.flag, c.half
                ));
            }
        }
    }
    for (t, ids) in m.tiles.iter().enumerate() {
        if !ids.is_empty() {
            let w = m.width as usize;
            let ids: Vec<String> = ids.iter().map(u32::to_string).collect();
            out.push(format!("tile {} {} {}", t % w, t / w, ids.join(" ")));
        }
    }
    out
}

/// A dump's lines with each contact's flag word cut to its byte: the game writes the flag as a
/// byte and leaves the word's upper bytes as the stack held them.
fn normalised(text: &str) -> Vec<String> {
    // the shapes' owners are the objects' (`mesh_owners_from_dump`), not the mesh's
    text.lines()
        .filter(|l| !l.starts_with("owner "))
        .map(|l| {
            let w: Vec<&str> = l.split_whitespace().collect();
            if w.first() == Some(&"side") && w.len() == 9 {
                let flag = u32::from_str_radix(w[7], 16).unwrap() & 0xff;
                format!(
                    "  side {} {} {} {} {} {} {flag:x} {}",
                    w[1], w[2], w[3], w[4], w[5], w[6], w[8]
                )
            } else {
                l.to_string()
            }
        })
        .collect()
}

#[test]
fn the_slice_s_mesh_operations_are_the_game_s() {
    let (Some(before), Some(ops)) = (
        std::env::var_os("GENIE_MESH"),
        std::env::var_os("GENIE_MESH_OPS"),
    ) else {
        return eprintln!("GENIE_MESH / GENIE_MESH_OPS are not set; skipping");
    };
    let before = std::fs::read_to_string(before).unwrap();
    let ops = std::fs::read_to_string(ops).unwrap();
    let mut mesh = genie_oracle::mesh_from_dump(&before).unwrap();
    // the dump itself reads back to the same text
    let read = normalised(&before);
    let back = dump(&mesh);
    let at = back.iter().zip(&read).position(|(a, b)| a != b);
    assert!(
        back == read,
        "the dump reads back differently at line {at:?}: {:?} against {:?} ({} lines against {})",
        at.map(|i| &back[i]),
        at.map(|i| &read[i]),
        back.len(),
        read.len()
    );
    let mut count = 0;
    for line in ops.lines().filter(|l| l.starts_with("op ")) {
        let words: Vec<&str> = line.split_whitespace().collect();
        let n = |i: usize| -> i32 { words[i].parse().unwrap() };
        match words[1] {
            "remove" => mesh.remove(n(2) as u32),
            "add" => {
                let got = mesh.add([n(2), n(3), n(4), n(5)]);
                assert_eq!(got, n(7) as u32, "{line}");
            }
            "replace" => {
                let got = mesh.replace(n(2) as u32, [n(3), n(4), n(5), n(6)]);
                assert_eq!(got, n(8) as u32, "{line}");
            }
            other => panic!("unknown operation {other}"),
        }
        count += 1;
    }
    let game: Vec<String> = normalised(&ops)
        .into_iter()
        .filter(|l| !l.starts_with("op "))
        .collect();
    let ours = dump(&mesh);
    let first = ours.iter().zip(&game).position(|(a, b)| a != b);
    assert!(
        ours == game,
        "after {count} operations the meshes differ first at line {first:?}: slice {:?} game {:?}",
        first.map(|i| &ours[i]),
        first.map(|i| &game[i])
    );
    assert!(100 <= count);
}
