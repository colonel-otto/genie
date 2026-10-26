//! What the pathing reads of a save and its dumps: the shape mesh and its owners, a unit's
//! terrain and the tile pathfinder's planner.

use crate::masters::TerrainTable;
use crate::{Error, live, spatial};
use genie_rec::de;

/// The obstruction manager's shape mesh as the tick oracle dumps it (`load_oracle/probes/mesh.py`):
/// a header line `mesh width W shapes N free F`, a `shape ID X0 Y0 X1 Y1` line per slot with its
/// sides' contacts (`  side S` and seven hex words), then `tile X Y IDS...` for every tile that
/// holds shapes. The save does not hold the mesh: its ids and orders follow the game's history.
///
/// # Panics
///
/// When a contact line of the dump does not hold numbers (the dump's own format).
#[must_use]
pub fn mesh_from_dump(text: &str) -> Option<genie_core::mesh::Mesh> {
    use genie_core::mesh::{Contact, Mesh};
    let mut mesh: Option<Mesh> = None;
    for line in text.lines() {
        let words: Vec<&str> = line.split_whitespace().collect();
        match words.first().copied() {
            Some("mesh") => {
                let width = words.get(2)?.parse().ok()?;
                let free = u32::from_str_radix(words.get(6)?.trim_start_matches("0x"), 16).ok()?;
                let mut m = Mesh::new(width);
                m.free = free;
                mesh = Some(m);
            }
            Some("shape") => {
                let m = mesh.as_mut()?;
                let v: Vec<i32> = words[2..6].iter().map(|w| w.parse().unwrap()).collect();
                m.shapes.push([v[0], v[1], v[2], v[3]]);
                m.sides.push(Default::default());
            }
            Some("side") => {
                let m = mesh.as_mut()?;
                let side: usize = words.get(1)?.parse().ok()?;
                let w: Vec<u32> = words[2..9]
                    .iter()
                    .map(|w| u32::from_str_radix(w, 16).unwrap())
                    .collect();
                m.sides.last_mut()?[side].push(Contact {
                    other: w[0],
                    side: w[1],
                    at: [w[2] as i32, w[3] as i32],
                    along: w[4] as i32,
                    flag: w[5] as u8,
                    half: w[6] as i32,
                });
            }
            Some("tile") => {
                let m = mesh.as_mut()?;
                let x: u32 = words.get(1)?.parse().ok()?;
                let y: u32 = words.get(2)?.parse().ok()?;
                m.tiles[(y * m.width + x) as usize] =
                    words[3..].iter().map(|w| w.parse().unwrap()).collect();
            }
            _ => {}
        }
    }
    mesh
}

/// The owner of each shape in a mesh dump (its `owner ID SHAPE` lines): the objects' `+0x64c`.
#[must_use]
pub fn mesh_owners_from_dump(text: &str) -> Vec<(i32, u32)> {
    text.lines()
        .filter_map(|line| {
            let mut w = line.split_whitespace();
            (w.next()? == "owner").then_some(())?;
            Some((w.next()?.parse().ok()?, w.next()?.parse().ok()?))
        })
        .collect()
}

/// What the tile pathfinder reads of object `id` in a save: its master's x radius and terrain
/// restriction (master `+0x5d8`, `+0x606`). None for an object the save does not hold.
pub fn unit_terrain(save: &[u8], id: i32) -> Result<Option<(f32, usize)>, Error> {
    let chapter = de::chapters(save)?[0];
    let header = chapter.header()?;
    for section in header.players() {
        for (_, s) in live(&section) {
            if s.id != id {
                continue;
            }
            let Some(m) = section
                .objects()
                .master(s.master_obj)
                .and_then(|m| m.static_part())
            else {
                return Ok(None);
            };
            return Ok(usize::try_from(m.terrain_restriction_mask_id)
                .ok()
                .map(|r| (m.radius_x, r)));
        }
    }
    Ok(None)
}

/// The tile pathfinder's planner for one terrain restriction (`passable`, its value per terrain),
/// built from a save's map and the tiles its static obstructions block, as the game builds it at
/// load (`FUN_140f7d310`). Checked against the game's grid, dumped by the tick oracle.
pub fn planner_from_save(
    save: &[u8],
    terrain: &TerrainTable,
    passable: Vec<f32>,
) -> Result<genie_core::tile_path::Planner, Error> {
    use genie_core::spatial::BLOCKED;
    let chapter = de::chapters(save)?[0];
    let header = chapter.header()?;
    let map = &header.world.base_world.map;
    let terrain_ids: Vec<u8> = map
        .rows
        .iter()
        .flat_map(|r| r.tiles.iter().map(|t| t.terrain_type))
        .collect();
    let spatial = spatial(&header, terrain);
    let tiles = spatial.tiles();
    let blocked = |x: i32, y: i32| tiles.word(x, y).is_some_and(|w| w & BLOCKED != 0);
    Ok(genie_core::tile_path::Planner::new(
        [map.map_width, map.map_height],
        terrain_ids,
        passable,
        &blocked,
    ))
}
