//! The shape mesh genie-core builds at the recording's header load (`Turns::build_mesh`) against
//! the game's right after its load (the tick oracle's mesh dump at update 0,
//! `load_oracle/probes/mesh.py` on a fork server at 0): every shape, side, tile list and owner.
//! Needs `GENIE_MESH_LOAD` (the dump), `GENIE_TICK_REC` and `GENIE_DAT`; skips without them.

mod common;

#[test]
fn the_load_builds_the_game_s_mesh() {
    let (Ok(dump), Ok(rec), Some(masters)) = (
        std::env::var("GENIE_MESH_LOAD"),
        std::env::var("GENIE_TICK_REC"),
        common::masters(),
    ) else {
        return eprintln!("GENIE_MESH_LOAD / GENIE_TICK_REC / GENIE_DAT are not set; skipping");
    };
    let rec = std::fs::read(rec).unwrap();
    let dump = std::fs::read_to_string(dump).unwrap();
    let turns = genie_oracle::header_run(&rec, 0, Some(&masters)).unwrap();
    let game = genie_oracle::mesh_from_dump(&dump).unwrap();
    let mine = turns.pathing.mesh().unwrap();
    assert_eq!(mine.shapes.len(), game.shapes.len());
    assert!(mine.shapes == game.shapes, "the shapes differ");
    assert!(mine.sides == game.sides, "the contacts differ");
    assert!(mine.tiles == game.tiles, "the tile lists differ");
    assert_eq!(mine.free, game.free);
    for (id, s) in genie_oracle::mesh_owners_from_dump(&dump) {
        assert_eq!(turns.pathing.shape(id), Some(s), "object {id}'s shape");
    }
}
