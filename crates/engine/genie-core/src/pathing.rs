//! The world's path planning state besides the objects' places: the tile pathfinder's planners
//! (`world+0x140`, one per terrain restriction), the obstruction manager's shape mesh
//! (`*(DAT_143f0a088 + 8)`) with each object's shape, and the short pather's own state.
//!
//! The mesh's ids and orders follow the game's history and are not in a save, so a slice started
//! from a save gets them from the game ([`Pathing::set_mesh`]); from then on the slice keeps them as
//! the game does: every position change of a tall object with an obstruction record updates its
//! shape (`RGE_Static_Object::new_position` `0x1411a58b0` -> `FUN_14118c540` -> `FUN_141056460`),
//! and every change of a static record's tiles rebuilds the planners' cells over them (the
//! manager's callback `FUN_140f7d4b0` -> `FUN_140f7e4b0`).

use crate::mesh::{Mesh, ONE, Rect};
use crate::short_path::Pather;
use crate::spatial::{BLOCKED, Placed, Tiles};
use crate::tile_path::Planner;
use std::collections::BTreeMap;

/// What a planner is built from and the planners built so far.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pathing {
    /// The map's size in tiles.
    pub size: [i32; 2],
    /// Each tile's terrain, row by row.
    pub terrain: Vec<u8>,
    /// Each terrain restriction's value per terrain (above 0 passes), by restriction id.
    pub restrictions: Vec<Vec<f32>>,
    /// The game type (the world's `game_type`): type 12 has the escape the motion step runs for
    /// a unit on an `E` tile (`FUN_141158210` result 1), which the slice does not model.
    pub game_type: u8,
    /// The restrictions the world's load gave planners (`FUN_140f7d310`, [`Pathing::register`]),
    /// in their order; None builds one for any restriction (a world made without a load).
    planned: Option<Vec<usize>>,
    planners: BTreeMap<usize, Planner>,
    /// Tile rectangles (x0, y0, x1, y1) whose static marks changed since the planners last saw
    /// them.
    pending: Vec<[i16; 4]>,
    /// The mesh, None until the slice has the game's.
    mesh: Option<Mesh>,
    /// Each object's shape (`+0x64c`).
    shapes: BTreeMap<i32, u32>,
    /// The temporary shapes of the motion step under way (its `local_1688`).
    temporary: Vec<u32>,
    /// The short pather.
    pub pather: Pather,
}

/// A shape's rectangle for an object (`FUN_141056460`): its box grown by half a tile, in fixed
/// point by truncation.
#[must_use]
pub fn shape_rect(p: &Placed) -> Rect {
    let [x, y, _] = p.position;
    let [rx, ry] = p.radius;
    let fixed = |v: f32| (v * ONE as f32) as i64 as i32;
    [
        fixed((x - rx) - 0.5),
        fixed((y - ry) - 0.5),
        fixed((x + rx) + 0.5),
        fixed((y + ry) + 0.5),
    ]
}

/// A shape's rectangle at the end of an action object's update (`FUN_141054fe0`): the place and
/// the radii each in fixed point by truncation, the box grown by half a tile (`0x200000`).
#[must_use]
pub fn refreshed_rect(position: [f32; 3], radius: [f32; 2]) -> Rect {
    let fixed = |v: f32| (v * ONE as f32) as i64 as i32;
    let [x, y, _] = position;
    let (rx, ry) = (fixed(radius[0]), fixed(radius[1]));
    let half = ONE / 2;
    [
        (fixed(x) - rx) - half,
        (fixed(y) - ry) - half,
        fixed(x) + half + rx,
        fixed(y) + half + ry,
    ]
}

impl Pathing {
    /// The planners and the pather for a map, without a mesh yet.
    #[must_use]
    pub fn new(
        size: [i32; 2],
        terrain: Vec<u8>,
        restrictions: Vec<Vec<f32>>,
        game_type: u8,
    ) -> Self {
        Self {
            size,
            terrain,
            restrictions,
            game_type,
            ..Self::default()
        }
    }

    /// An empty mesh for a map `width` tiles wide, as the game's load starts it.
    pub fn start_mesh(&mut self, width: u32) {
        self.mesh = Some(Mesh::new(width));
        self.shapes.clear();
        self.temporary.clear();
    }

    /// The game's mesh and each object's shape in it, at the tick the slice starts from.
    pub fn set_mesh(&mut self, mesh: Mesh, shapes: impl IntoIterator<Item = (i32, u32)>) {
        self.mesh = Some(mesh);
        self.shapes = shapes.into_iter().collect();
    }

    /// The mesh, once set.
    #[must_use]
    pub const fn mesh(&self) -> Option<&Mesh> {
        self.mesh.as_ref()
    }

    /// The mesh and the short pather together, for a search.
    pub fn mesh_and_pather(&mut self) -> Option<(&mut Mesh, &mut Pather)> {
        Some((self.mesh.as_mut()?, &mut self.pather))
    }

    /// An object's shape in the mesh.
    #[must_use]
    pub fn shape(&self, id: i32) -> Option<u32> {
        self.shapes.get(&id).copied()
    }

    /// The world's load gives restriction `restriction` a planner (`FUN_140f7d310`), unless one
    /// with the same values has one.
    pub fn register(&mut self, restriction: usize) {
        let planned = self.planned.get_or_insert_with(Vec::new);
        let values = self.restrictions.get(restriction);
        if values.is_some() && !planned.iter().any(|&r| self.restrictions.get(r) == values) {
            planned.push(restriction);
        }
    }

    /// The planner of a terrain restriction (`FUN_140f7cb30` -> `FUN_140f7ca70`): the first of
    /// the load's planners whose restriction has the same values, else none (the slice builds it
    /// from the map on first use, `FUN_140f74f90`); the planners built before see the tiles'
    /// changes first. None for a restriction the data has not got.
    pub fn planner(&mut self, restriction: usize, tiles: &Tiles) -> Option<&mut Planner> {
        let blocked = |x: i32, y: i32| tiles.word(x, y).is_some_and(|w| w & BLOCKED != 0);
        for rect in self.pending.drain(..) {
            for p in self.planners.values_mut() {
                p.update(rect, &blocked);
            }
        }
        let values = self.restrictions.get(restriction)?;
        let key = match self.planned.as_ref() {
            None => restriction,
            Some(planned) => planned
                .iter()
                .copied()
                .find(|&r| self.restrictions.get(r) == Some(values))?,
        };
        if !self.planners.contains_key(&key) {
            let passable = self.restrictions.get(key)?.clone();
            let planner = Planner::new(self.size, self.terrain.clone(), passable, &blocked);
            let _ = self.planners.insert(key, planner);
        }
        self.planners.get_mut(&key)
    }

    /// Static marks changed over these tile rectangles.
    pub fn tiles_changed(&mut self, rects: impl IntoIterator<Item = [i16; 4]>) {
        self.pending.extend(rects);
    }

    /// The object's position changed (`FUN_141056460`'s shape part, through `FUN_14118c540`): a
    /// tall object with a record of a kind other than 0 and 12 and a class other than 0 has a shape
    /// while it is not moving (object slot 185, `0x1411583d0`), added or replaced
    /// (`FUN_140fdb8d0`, `FUN_140fdc730`), and none while it moves (`FUN_140fdc250`). Without the
    /// game's mesh nothing is kept.
    pub fn moved(&mut self, id: i32, p: &Placed, moving: bool) {
        let Some(mesh) = self.mesh.as_mut() else {
            return;
        };
        let has_shape = p.tall
            && p.obstruction
                .is_some_and(|o| o.kind != 0 && o.kind != 12 && o.class != 0);
        if !has_shape {
            return;
        }
        if moving {
            if let Some(s) = self.shapes.remove(&id) {
                mesh.remove(s);
            }
            return;
        }
        let rect = shape_rect(p);
        let s = match self.shapes.get(&id) {
            None => mesh.add(rect),
            Some(&s) => mesh.replace(s, rect),
        };
        let _ = self.shapes.insert(id, s);
    }

    /// `FUN_141056460`'s shape for an object that does not move, by its master's obstruction
    /// kind and class whether or not it has its record (the load's rehook): added or replaced.
    pub fn place_shape(&mut self, id: i32, p: &Placed) {
        let Some(mesh) = self.mesh.as_mut() else {
            return;
        };
        let rect = shape_rect(p);
        let s = match self.shapes.get(&id) {
            None => mesh.add(rect),
            Some(&s) => mesh.replace(s, rect),
        };
        let _ = self.shapes.insert(id, s);
    }

    /// The end of an action object's update (`RGE_Action_Object::update` -> `FUN_141163080` ->
    /// `FUN_141055270` -> `FUN_141054fe0`), every tick: an object with an obstruction record has a
    /// shape while it is not moving (slot 185), its box (`outline`, the master's `+0x688`,
    /// `+0x68c`, for obstruction type 11, else its radii) grown by half a tile
    /// ([`refreshed_rect`]), added or replaced (an odd box always replaces: its shape is removed
    /// and added again); none while it moves. The record's moving flag (byte 3, 0x10) is not
    /// held: the walk reads the gait. Without the game's mesh nothing is kept.
    pub fn refresh(&mut self, id: i32, p: &Placed, moving: bool, outline: [f32; 2]) {
        let Some(mesh) = self.mesh.as_mut() else {
            return;
        };
        if p.obstruction.is_none() {
            return;
        }
        if moving {
            if let Some(s) = self.shapes.remove(&id) {
                mesh.remove(s);
            }
            return;
        }
        let radius = if p.obstruction_type == 11 {
            outline
        } else {
            p.radius
        };
        let rect = refreshed_rect(p.position, radius);
        let s = match self.shapes.get(&id) {
            None => mesh.add(rect),
            Some(&s) => mesh.replace(s, rect),
        };
        let _ = self.shapes.insert(id, s);
    }

    /// A temporary shape for a kept hit of the walk (`FUN_141054fe0` with 1 on an object without a
    /// shape, which the object does not keep): [`refreshed_rect`] of its radii.
    pub fn add_temporary_shape(&mut self, p: &Placed) {
        if let Some(mesh) = self.mesh.as_mut() {
            let s = mesh.add(refreshed_rect(p.position, p.radius));
            self.temporary.push(s);
        }
    }

    /// The motion step's exit (`FUN_141055970` on each): the temporary shapes leave the mesh in
    /// the order they came.
    pub fn drop_temporary_shapes(&mut self) {
        let shapes = std::mem::take(&mut self.temporary);
        if let Some(mesh) = self.mesh.as_mut() {
            for s in shapes {
                mesh.remove(s);
            }
        }
    }

    /// The object left the world: its shape leaves the mesh.
    pub fn removed(&mut self, id: i32) {
        if let (Some(mesh), Some(s)) = (self.mesh.as_mut(), self.shapes.remove(&id)) {
            mesh.remove(s);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spatial::Obstruction;

    fn villager(x: f32, y: f32) -> Placed {
        Placed {
            position: [x, y, 0.0],
            radius: [0.2, 0.2],
            class: 4,
            player: 2,
            master: 118,
            obstruction_type: 5,
            buildable_on: 0,
            tall: true,
            speed: 0.8,
            unknown: None,
            obstruction: Some(Obstruction { kind: 5, class: 2 }),
        }
    }

    #[test]
    fn a_unit_s_shape_is_its_box_grown_by_half_a_tile() {
        let r = shape_rect(&villager(30.0, 89.0));
        assert_eq!(
            r,
            [
                (29.3f32 * 4_194_304.0) as i32,
                (88.3f32 * 4_194_304.0) as i32,
                (30.7f32 * 4_194_304.0) as i32,
                (89.7f32 * 4_194_304.0) as i32
            ]
        );
    }

    #[test]
    /// MarvinBlue/426048731's update 1141: the hunter 10029 at (106.53662, 46.65576), standing,
    /// gets its shape at its update's end, the box of its radii (0.2) grown by half a tile.
    fn a_standing_unit_gets_its_shape_at_its_update_s_end() {
        let mut p = Pathing::default();
        p.set_mesh(Mesh::new(120), []);
        let hunter = villager(106.53662, 46.65576);
        p.refresh(10029, &hunter, false, [0.0; 2]);
        let s = p.shape(10029).unwrap();
        let r = refreshed_rect(hunter.position, hunter.radius);
        assert_eq!(
            r[0],
            (106.53662f32 * 4_194_304.0) as i32 - 838_860 - 0x20_0000
        );
        assert_eq!(
            p.mesh().unwrap().shapes[s as usize],
            [r[0] & !1, r[1] & !1, (r[2] + 1) & !1, (r[3] + 1) & !1]
        );
        // moving, it has none; without a record, none either
        p.refresh(10029, &hunter, true, [0.0; 2]);
        assert_eq!(p.shape(10029), None);
        let mut bare = hunter;
        bare.obstruction = None;
        p.refresh(10029, &bare, false, [0.0; 2]);
        assert_eq!(p.shape(10029), None);
    }

    #[test]
    fn a_unit_has_a_shape_while_it_stands_and_none_while_it_moves() {
        let mut p = Pathing::default();
        p.set_mesh(Mesh::new(120), []);
        p.moved(9175, &villager(30.0, 89.0), false);
        let s = p.shape(9175).unwrap();
        assert_eq!(p.mesh().unwrap().shapes[s as usize], {
            let r = shape_rect(&villager(30.0, 89.0));
            [r[0] & !1, r[1] & !1, (r[2] + 1) & !1, (r[3] + 1) & !1]
        });
        p.moved(9175, &villager(30.1, 89.0), true);
        assert_eq!(p.shape(9175), None);
        // a foundation without its record has none
        let mut site = villager(30.0, 88.0);
        site.obstruction = None;
        p.moved(10028, &site, false);
        assert_eq!(p.shape(10028), None);
    }
}
